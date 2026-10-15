{ inputs, ... }:
{
  flake.overlays.default =
    final: _:
    let
      mkFanta = import ../toolchain.nix { inherit inputs; };
    in
    {
      fanta-editor = mkFanta final;
    };
}
