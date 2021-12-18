{
  description = "wusic: A music storage system";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  inputs.naersk.url = "github:nmattia/naersk";

  outputs = { self, nixpkgs, flake-utils, naersk }:
    flake-utils.lib.eachDefaultSystem
      (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfree = true;
          };
          naersk-lib = naersk.lib."${system}";
        in
        rec {
          packages."wusic" =
            let
              llvmPackages = pkgs.llvmPackages_latest;
              cargoToml = (builtins.fromTOML (builtins.readFile ./Cargo.toml));
            in
            naersk-lib.buildPackage {
              pname = cargoToml.package.name;
              version = cargoToml.package.version;

              src = ./.;

              nativeBuildInputs = with pkgs; [
                pkg-config

                llvmPackages.llvm
                llvmPackages.clang
              ];

              buildInputs = with pkgs; [
                stdenv.cc.cc
                stdenv.cc.libc

                ffmpeg
              ];

              LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

              preBuildPhase = with pkgs; ''
                BINDGEN_CFLAGS="$(< ${stdenv.cc}/nix-support/libc-crt1-cflags) \
                  $(< ${stdenv.cc}/nix-support/libc-cflags) \
                  $(< ${stdenv.cc}/nix-support/cc-cflags) \
                  $(< ${stdenv.cc}/nix-support/libcxx-cxxflags) \
                  ${lib.optionalString stdenv.cc.isClang "-idirafter ${stdenv.cc.cc.lib}/lib/clang/${lib.getVersion stdenv.cc.cc}/include"} \
                  ${lib.optionalString stdenv.cc.isGNU "-isystem ${lib.getDev stdenv.cc.cc}/include/c++/${lib.getVersion stdenv.cc.cc} -isystem ${stdenv.cc.cc}/include/c++/${lib.getVersion stdenv.cc.cc}/${stdenv.hostPlatform.config}"} \
                  $NIX_CFLAGS_COMPILE"
              '';
            };

          defaultPackage = packages."wusic";

          devShell = pkgs.mkShell {
            buildInputs = with pkgs; [
              pkg-config
              stdenv.cc.cc
              stdenv.cc.libc

              llvmPackages_latest.llvm
              llvmPackages_latest.clang

              cargo
              rustc

              ffmpeg
            ];

            LIBCLANG_PATH = "${pkgs.llvmPackages_latest.libclang.lib}/lib";

            shellHook = with pkgs; ''
              export BINDGEN_CFLAGS="$(< ${stdenv.cc}/nix-support/libc-crt1-cflags) \
                $(< ${stdenv.cc}/nix-support/libc-cflags) \
                $(< ${stdenv.cc}/nix-support/cc-cflags) \
                $(< ${stdenv.cc}/nix-support/libcxx-cxxflags) \
                ${lib.optionalString stdenv.cc.isClang "-idirafter ${stdenv.cc.cc.lib}/lib/clang/${lib.getVersion stdenv.cc.cc}/include"} \
                ${lib.optionalString stdenv.cc.isGNU "-isystem ${lib.getDev stdenv.cc.cc}/include/c++/${lib.getVersion stdenv.cc.cc} -isystem ${stdenv.cc.cc}/include/c++/${lib.getVersion stdenv.cc.cc}/${stdenv.hostPlatform.config}"} \
                $NIX_CFLAGS_COMPILE"
            '';
          };
        }
      );
}
