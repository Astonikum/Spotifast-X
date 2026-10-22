{
  description = "Spotify, native and fast";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # rust-toolchain.toml pins the compiler so local builds and CI agree.
    # This reads that file rather than restating the version here.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
              rust-analyzer
              pkg-config
            ]
            ++ lib.optionals stdenv.hostPlatform.isDarwin [
              apple-sdk
            ]
            ++ lib.optionals stdenv.hostPlatform.isLinux [
              pkgs.rustPlatform.bindgenHook
              dbus
              alsa-lib
              pipewire
              libxkbcommon
              wayland
              libGL
              libx11
              libxcursor
              libxi
              libxrandr
            ];
          # The GUI dlopens its Wayland, X11 and GL libraries at run time.
          LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
            pkgs.lib.makeLibraryPath (
              with pkgs;
              [
                dbus
                libxkbcommon
                wayland
                libGL
                libx11
                libxcursor
                libxi
                libxrandr
              ]
            )
          );
        };
      });

      packages = forAllSystems (
        pkgs:
        let
          spotifast =
            let
              toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
              rustPlatform = pkgs.makeRustPlatform {
                cargo = toolchain;
                rustc = toolchain;
              };
              runtimeLibs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
                with pkgs;
                [
                  dbus
                  libxkbcommon
                  wayland
                  libGL
                  libx11
                  libxcursor
                  libxi
                  libxrandr
                ]
              );
            in
            rustPlatform.buildRustPackage rec {
              pname = "spotifast-x";
              version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
              src = self;

              # The lock file contains git dependencies. fetchCargoVendor includes
              # them in the fixed-output dependency tree, unlike cargoLock alone.
              cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
                pname = "spotifast-x";
                version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
                src = self;
                hash = "sha256-lISYWmlHSRYFp4CBulizJN+Rj6tYFfPrT4fw4wpvrrw=";
              };
              nativeBuildInputs =
                with pkgs;
                [
                  pkg-config
                ]
                ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                  pkgs.rustPlatform.bindgenHook
                  makeWrapper
                ]
                ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
                  rcodesign
                  icnsify
                ];
              # The command-alias regression starts its own bus, isolated from
              # the listener's desktop and running application.
              nativeCheckInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.dbus ];
              buildInputs =
                pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
                  with pkgs;
                  [
                    alsa-lib
                    pipewire
                    libGL
                    libx11
                  ]
                )
                ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.apple-sdk ];

              # Nix's Rust check hook deliberately points SSL_CERT_FILE at a
              # missing path. The librespot proxy regression test constructs a
              # TLS connector before asserting its CONNECT request, so give the
              # test an explicit, sandboxed trust store.
              preCheck = ''
                export SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              '';

              # The GUI dlopens its Wayland, X11 and GL libraries at run time.
              postFixup =
                pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                  wrapProgram $out/bin/spotifast-x \
                    --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
                ''
                + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  rcodesign sign "$out/Applications/Spotifast X.app"
                '';

              postInstall = ''
                mv "$out/bin/spotifast" "$out/bin/spotifast-x"
              '' + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                  install -Dm644 packaging/applications/spotifast-x.desktop \
                    $out/share/applications/spotifast-x.desktop
                  install -Dm644 packaging/icons/spotifast-x.svg \
                    $out/share/icons/hicolor/scalable/apps/spotifast-x.svg
                  install -Dm644 contrib/omarchy/spotifast.json.tpl \
                    $out/share/spotifast-x/omarchy/spotifast.json.tpl
                  install -Dm755 contrib/omarchy/spotifast-theme \
                    $out/share/spotifast-x/omarchy/spotifast-theme
                ''
                + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  app="$out/Applications/Spotifast X.app/Contents"
                  mkdir -p "$app/MacOS" "$app/Resources"
                  executable=Spotifast
                  identifier=io.github.Astonikum.SpotifastX
                  cp "$out/bin/spotifast-x" "$app/MacOS/$executable"
                  icnsify packaging/macos/icon-1024.png -o "$app/Resources/spotifast.icns"
                  substitute packaging/macos/Info.plist "$app/Info.plist" \
                    --replace-fail __VERSION__ "${version}" \
                    --replace-fail __BUILD__ "${pkgs.lib.head (pkgs.lib.splitString "-" version)}" \
                    --replace-fail __EXECUTABLE__ "$executable" \
                    --replace-fail __IDENTIFIER__ "$identifier"
                '';

              meta = {
                description = "Fast native Spotify client with local playback and Spotify Connect";
                homepage = "https://github.com/Astonikum/Spotifast-X";
                license = pkgs.lib.licenses.mit;
                mainProgram = "spotifast-x";
              };
            };

        in
        {
          default = spotifast;
          inherit spotifast;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
          spotifast-app = spotifast;
        }
      );

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
