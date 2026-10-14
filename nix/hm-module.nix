self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.tmux-agent-notifier;
  tan = lib.getExe cfg.package;
  # Stable across rebuilds, unlike the store path, so agent configs don't churn.
  hookBin = "${config.home.profileDirectory}/bin/tan";

  agentNames = [
    "claude"
    "cursor"
    "goose"
  ];
  enabledAgents = lib.filter (a: cfg.agents.${a}.enable) agentNames;
  disabledAgents = lib.filter (a: !cfg.agents.${a}.enable) agentNames;

  needsUser = "#{||:#{==:#{@agent_state},waiting},#{==:#{@agent_state},attention}}";

  tmuxConfig = ''
    # tmux-agent-notifier
    set -g focus-events on
  ''
  + lib.optionalString (cfg.tmux.desktop == "osc") ''
    # Passthrough from panes that aren't visible, so background agents can notify.
    set -g allow-passthrough all
  ''
  + ''
    set -g @tan_desktop ${cfg.tmux.desktop}
    set -g @tan_bell ${if cfg.tmux.bell then "on" else "off"}
    set -g @tan_message ${if cfg.tmux.message then "on" else "off"}
    set-hook -ga pane-focus-in 'run-shell -b "${tan} clear --pane #{pane_id}"'
  ''
  + lib.optionalString (cfg.tmux.pickerKey != null) ''
    bind-key ${cfg.tmux.pickerKey} choose-tree -Zw -f '${needsUser}'
  ''
  + lib.optionalString (cfg.tmux.nextKey != null) ''
    bind-key ${cfg.tmux.nextKey} run-shell -b '${tan} next --client "#{client_name}"'
  ''
  + lib.optionalString cfg.tmux.statusRight ''
    set -ag status-right ' #(${tan} status)'
  '';
in
{
  options.programs.tmux-agent-notifier = {
    enable = lib.mkEnableOption "tmux notifications for coding agents";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "tmux-agent-notifier.packages.\${system}.default";
      description = "The tmux-agent-notifier package.";
    };

    agents = lib.genAttrs agentNames (name: {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Register hooks with ${name}. Disabling removes previously installed hooks.";
      };
    });

    tmux = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = config.programs.tmux.enable;
        defaultText = lib.literalExpression "config.programs.tmux.enable";
        description = "Append tmux configuration (focus clearing, key bindings) to programs.tmux.";
      };
      desktop = lib.mkOption {
        type = lib.types.enum [
          "osc"
          "osascript"
          "off"
        ];
        default = "osc";
        description = ''
          Desktop notification channel. `osc` sends OSC 777 through tmux
          passthrough (Ghostty, WezTerm, kitty); `osascript` uses macOS
          Notification Center directly.
        '';
      };
      bell = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Ring the pane's bell so tmux flags the window.";
      };
      message = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Show a tmux message on clients not viewing the agent's window.";
      };
      pickerKey = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = "a";
        description = "Prefix key for a tree picker of panes waiting on you.";
      };
      nextKey = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = "A";
        description = "Prefix key that jumps to the most urgent waiting pane.";
      };
      statusRight = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Append `tan status` to status-right. Themes that generate
          status-right in their plugin script may override this.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    programs.tmux.extraConfig = lib.mkIf cfg.tmux.enable (lib.mkAfter tmuxConfig);

    home.activation.tmuxAgentNotifier = lib.hm.dag.entryAfter [ "writeBoundary" ] (
      lib.optionalString (enabledAgents != [ ]) ''
        run ${tan} install --bin ${lib.escapeShellArg hookBin} ${lib.concatStringsSep " " enabledAgents}
      ''
      + lib.optionalString (disabledAgents != [ ]) ''
        run ${tan} uninstall ${lib.concatStringsSep " " disabledAgents}
      ''
    );
  };
}
