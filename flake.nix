{
  description = "failover-forge — Akeyless region failover drill orchestrator (containerized for K8s CronJobs)";

  nixConfig = {
    allow-import-from-derivation = true;
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    crate2nix.url = "github:nix-community/crate2nix";
    flake-utils.url = "github:numtide/flake-utils";
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    forge = {
      url = "github:pleme-io/forge";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crate2nix, flake-utils, substrate, forge, ... }:
    (import "${substrate}/lib/build/rust/tool-image-flake.nix" {
      inherit nixpkgs crate2nix flake-utils forge;
    }) {
      toolName = "failover-forge";
      src = self;
      repo = "pleme-io/failover-forge";
      tag = "0.1.0";
      # Use substrate's default architectures (amd64 + arm64). Per
      # substrate/lib/service/image-release.nix the standard tagging is
      # `<arch>-<git-sha>` + `<arch>-latest` per architecture. The
      # multi-arch manifest combine step (regctl manifest put) currently
      # fails due to a substrate/regctl CLI mismatch — tracked in task #38
      # — but the per-arch tags are pushed successfully BEFORE that step,
      # so the canonical references still exist.
      # Runtime tools the failover drill subprocess-invokes:
      #   curl     — continuous HTTP probe loop + Confluence publishing
      #   gh       — optional trigger of existing akeyless GH workflows
      #   tar/gzip — tarball assembly
      #   cacert   — HTTPS trust bundle for the probe + gh
      extraContents = pkgs: with pkgs; [
        cacert
        curl
        gh
        gnutar
        gzip
        coreutils
      ];
    };
}
