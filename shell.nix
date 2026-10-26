{ pkgs ? import <nixpkgs> { } }:
let
  # CEF as cef-rs expects to find it (`CEF_PATH`): the Release and Resources files side by
  # side, and an archive.json naming the version. The cef crate's version follows it.
  cef = pkgs.runCommand "quill-cef-${pkgs.cef-binary.version}" { } ''
    mkdir $out
    ln -s ${pkgs.cef-binary}/Release/* ${pkgs.cef-binary}/Resources/* $out/
    for name in CMakeLists.txt cmake include libcef_dll CREDITS.html; do
      ln -s ${pkgs.cef-binary}/$name $out/$name
    done
    echo '{"type":"minimal","name":"cef_binary_${pkgs.cef-binary.version}+nixpkgs","sha1":""}' > $out/archive.json
  '';
in
pkgs.mkShell {
  # cmake and ninja build CEF's C++ wrapper for cef-rs.
  nativeBuildInputs = with pkgs; [ stdenv.cc pkg-config cmake ninja ];
  buildInputs = with pkgs; [ wayland libxkbcommon ];
  CEF_PATH = "${cef}";
  # winit and wgpu dlopen these at runtime; libcef.so is linked from CEF_PATH.
  LD_LIBRARY_PATH = "${cef}:" + pkgs.lib.makeLibraryPath (with pkgs; [ wayland libxkbcommon vulkan-loader libGL libx11 libxcursor libxi libxrandr ]);
}
