//! The release side of quill's updates, for the owner's machine and the
//! release workflow (`docs/UPDATES.md`):
//!
//! ```text
//! quill-release keygen <private-key-file>   a new key: the private half to the file, the public to stdout
//! quill-release tag <tag>                   the tag is annotated and names this workspace's version
//! quill-release manifest <tag> <base-url> <platform>=<file>...
//!                                          the manifest of the release, to stdout
//! quill-release sign <manifest>             <manifest>.sig, with QUILL_UPDATE_SIGNING_KEY
//! quill-release verify <manifest>           <manifest>.sig verifies with the key compiled in
//! ```
//!
//! The private key is 32 bytes in base64 and never printed.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode};

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use quill_update::{Artifact, Manifest, Version};
use sha2::{Digest, Sha256};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let result = match arguments.as_slice() {
        ["keygen", file] => keygen(Path::new(file)),
        ["tag", tag] => tag_version(tag).map(|version| println!("{version}")),
        ["manifest", tag, base, artifacts @ ..] if !artifacts.is_empty() => {
            manifest(tag, base, artifacts)
        }
        ["sign", file] => sign(Path::new(file)),
        ["verify", file] => verify(Path::new(file)),
        _ => Err("usage: quill-release keygen <file> | tag <tag> | manifest <tag> <base-url> <platform>=<file>... | sign <manifest> | verify <manifest>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("quill-release: {error}");
            ExitCode::FAILURE
        }
    }
}

type Result<T> = std::result::Result<T, String>;

fn base64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn keygen(file: &Path) -> Result<()> {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|error| error.to_string())?;
    let key = SigningKey::from_bytes(&seed);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut out = options
        .open(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    writeln!(out, "{}", base64().encode(seed)).map_err(|error| error.to_string())?;
    println!("{}", base64().encode(key.verifying_key().to_bytes()));
    Ok(())
}

fn git(arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(arguments)
        .output()
        .map_err(|error| format!("git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

/// Every annotated tag: its name and message.
fn annotated_tags() -> Result<Vec<(String, String)>> {
    let listed = git(&[
        "for-each-ref",
        "refs/tags",
        "--format=%(refname:strip=2)%00%(objecttype)%00%(contents)%00",
    ])?;
    let fields: Vec<&str> = listed.split('\0').collect();
    Ok(fields
        .chunks_exact(3)
        .filter(|tag| tag[1] == "tag")
        .map(|tag| (tag[0].trim().to_owned(), tag[2].trim().to_owned()))
        .collect())
}

/// The version `tag` names, once it is annotated and names this
/// workspace's version: a release from a forgotten bump would offer
/// itself again forever.
fn tag_version(tag: &str) -> Result<Version> {
    if !annotated_tags()?.iter().any(|(name, _)| name == tag) {
        return Err(format!(
            "{tag} is not an annotated tag: git tag -a {tag} -m \"…\""
        ));
    }
    let version = Version::parse(tag.strip_prefix('v').ok_or("a tag starts with v")?)
        .map_err(|error| format!("{tag}: {error}"))?;
    if version.to_string() != quill_update::VERSION {
        return Err(format!(
            "{tag} does not match the workspace's version {}",
            quill_update::VERSION
        ));
    }
    Ok(version)
}

fn manifest(tag: &str, base: &str, artifacts: &[&str]) -> Result<()> {
    let version = tag_version(tag)?;
    let tags = annotated_tags()?;
    let notes = tags
        .iter()
        .find(|(name, _)| name == tag)
        .map(|(_, message)| message.clone())
        .unwrap_or_default();
    let minimum_version = quill_update::minimum_version(
        &version,
        tags.iter()
            .map(|(name, message)| (name.as_str(), message.as_str())),
    );
    let date = git(&[
        "for-each-ref",
        &format!("refs/tags/{tag}"),
        "--format=%(taggerdate:iso-strict)",
    ])?
    .trim()
    .to_owned();
    let mut platforms = std::collections::BTreeMap::new();
    for artifact in artifacts {
        let (platform, file) = artifact
            .split_once('=')
            .ok_or_else(|| format!("{artifact}: <platform>=<file>"))?;
        let bytes = std::fs::read(file).map_err(|error| format!("{file}: {error}"))?;
        let name = Path::new(file)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{file}: no file name"))?;
        platforms.insert(
            platform.to_owned(),
            Artifact {
                url: format!("{}/{name}", base.trim_end_matches('/')),
                size: bytes.len() as u64,
                sha256: quill_update::hex(&Sha256::digest(&bytes)),
            },
        );
    }
    let manifest = Manifest {
        version,
        date,
        notes,
        minimum_version,
        platforms,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn signature_path(file: &Path) -> std::path::PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".sig");
    name.into()
}

fn sign(file: &Path) -> Result<()> {
    let seed = std::env::var("QUILL_UPDATE_SIGNING_KEY")
        .map_err(|_| "QUILL_UPDATE_SIGNING_KEY is not set")?;
    let seed: [u8; 32] = base64()
        .decode(seed.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("QUILL_UPDATE_SIGNING_KEY is not a key from `quill-release keygen`")?;
    let key = SigningKey::from_bytes(&seed);
    let bytes = std::fs::read(file).map_err(|error| format!("{}: {error}", file.display()))?;
    // Only what quill would accept is signed.
    serde_json::from_slice::<Manifest>(&bytes)
        .map_err(|error| format!("not a manifest: {error}"))?;
    let signature = base64().encode(key.sign(&bytes).to_bytes());
    std::fs::write(signature_path(file), format!("{signature}\n"))
        .map_err(|error| error.to_string())
}

fn verify(file: &Path) -> Result<()> {
    let key = quill_update::release_key()
        .ok_or("no release key compiled in: run quill-release keygen and commit crates/quill-update/release-key.pub")?;
    let bytes = std::fs::read(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let signature = std::fs::read_to_string(signature_path(file))
        .map_err(|error| format!("{}.sig: {error}", file.display()))?;
    let manifest = quill_update::verify(&bytes, &signature, &key).map_err(|error| {
        format!("{error:?}: it does not verify with release-key.pub (another signing secret, or a changed manifest)")
    })?;
    println!(
        "{} verified (minimum {})",
        manifest.version,
        manifest
            .minimum_version
            .map_or("none".into(), |minimum| minimum.to_string())
    );
    Ok(())
}
