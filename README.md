# keyboard-swap

CLI to switch a VIA-enabled QMK keyboard between JSON layouts exported from [usevia.app](https://usevia.app).

Built with [Bastard Keyboard's Charybdis](https://bastardkb.com/charybdis/) in mind, but should be generalized.

## Usage

```sh
keyboard-swap            # cycle to the next layout
keyboard-swap gamer      # switch to a layout by name
keyboard-swap 2          # ...or by 1-based position
keyboard-swap --list     # list layouts; * marks the one on the keyboard
keyboard-swap -n gamer   # dry run: show what would change
```

## Config

`~/.config/keyboard-swap/config.toml` (or `$XDG_CONFIG_HOME/...`, or `--config <path>`):

```toml
layouts = [
    "~/Documents/layouts/standard.json", # named "standard", from the file name
    { path = "~/Documents/layouts/gamer.json", name = "gaming" },
]
```

Relative paths are resolved against the config file's directory. All layouts in one config must
be for the same keyboard (they're matched by the `vendorProductId` in the export).

## Permissions (Linux)

By default only root can open hidraw devices. Install a udev rule once:

```sh
/path/to/keyboard-swap --udev-rule | sudo tee /etc/udev/rules.d/50-keyboard-swap.rules
sudo udevadm control --reload && sudo udevadm trigger
```

Then unplug and replug the keyboard. This also lets VIA in the browser work without sudo.

## Contact

benjamin.w.massey@gmail.com

## License

This project is licensed under the [GNU General Public License (GPLv3)](https://www.gnu.org/licenses/gpl-3.0.en.html).

A copy of said license can be found in `LICENSE.md`.
