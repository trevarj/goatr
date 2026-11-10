{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.goatr;
in
{
  options.services.goatr = {
    enable = lib.mkEnableOption "the manually started Goatr companion user service";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.goatr;
      defaultText = lib.literalExpression "inputs.goatr.packages.\${pkgs.stdenv.hostPlatform.system}.goatr";
      description = "Goatr package providing the companion and CLI.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    systemd.user.services.goatr-companion = {
      description = "Goatr full-control Herdr companion (manual start)";
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} serve --hc";
        Restart = "on-failure";
        RestartSec = 5;
        UMask = "0077";
      };
    };
  };
}
