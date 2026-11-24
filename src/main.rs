mod config;
mod layout;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use layout::Layout;
use qmk_via_api::{KeyboardApi, MatrixInfo};
use std::path::PathBuf;

/// Raw HID usage page that VIA-enabled QMK firmware listens on.
const VIA_USAGE_PAGE: u16 = 0xFF60;
/// VIA protocol versions: 8 added bulk keymap transfers, 12 switched to QMK 0.19+ keycodes.
const MIN_PROTOCOL: u16 = 8;
const NEW_KEYCODES_PROTOCOL: u16 = 12;

/// Swap a VIA-enabled keyboard between layouts exported from usevia.app.
///
/// With no argument, switches to the layout after the one currently on the keyboard.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Layout to switch to: its name, or its 1-based position in the config.
    target: Option<String>,

    /// Config file (default: ~/.config/keyboard-swap/config.toml).
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// List configured layouts and show which one is on the keyboard.
    #[arg(short, long, conflicts_with_all = ["target", "dry_run", "udev_rule"])]
    list: bool,

    /// Show what would change without writing to the keyboard.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Print a udev rule that lets non-root users talk to the keyboard, then exit.
    #[arg(long, conflicts_with_all = ["target", "dry_run"])]
    udev_rule: bool,
}

/// What is currently stored on the keyboard.
struct Board {
    keymap: Vec<Vec<u16>>,
    macros: Vec<Vec<u8>>,
}

/// One layout, encoded for the connected keyboard.
struct Encoded {
    keymap: Vec<Vec<u16>>,
    macros: Vec<Vec<u8>>,
}

impl Encoded {
    fn keys_differing(&self, board: &Board) -> usize {
        self.keymap
            .iter()
            .zip(&board.keymap)
            .map(|(want, have)| want.iter().zip(have).filter(|(a, b)| a != b).count())
            .sum()
    }

    fn macros_differ(&self, board: &Board) -> bool {
        self.macros.iter().enumerate().any(|(i, m)| board.macros.get(i) != Some(m))
    }

    fn matches(&self, board: &Board) -> bool {
        self.keys_differing(board) == 0 && !self.macros_differ(board)
    }
}

fn via(e: qmk_via_api::Error) -> anyhow::Error {
    anyhow!("{e}")
}

fn matrix(keys: usize) -> Result<MatrixInfo> {
    // Only rows * cols matters for VIA's bulk transfers, so any factorisation will do.
    let cols = (1..=255usize)
        .rev()
        .find(|c| keys.is_multiple_of(*c) && keys / c <= 255)
        .with_context(|| format!("can't handle {keys} keys per layer"))?;
    Ok(MatrixInfo { rows: (keys / cols) as u8, cols: cols as u8 })
}

fn read_board(api: &KeyboardApi, layers: usize, keys: usize, macro_count: usize) -> Result<Board> {
    let info = matrix(keys)?;
    let keymap = (0..layers)
        .map(|l| api.read_raw_matrix(info, l as u8).map_err(via))
        .collect::<Result<_>>()?;
    let bytes = if macro_count > 0 { api.get_macro_bytes().map_err(via)? } else { vec![] };
    let macros = bytes.split(|&b| b == 0).take(macro_count).map(<[u8]>::to_vec).collect();
    Ok(Board { keymap, macros })
}

fn pick_target(target: Option<&str>, layouts: &[Layout], current: Option<usize>) -> Result<usize> {
    let Some(target) = target else {
        return Ok(current.map_or(0, |c| (c + 1) % layouts.len()));
    };
    if let Some(i) = layouts.iter().position(|l| l.name == target) {
        return Ok(i);
    }
    match target.parse::<usize>() {
        Ok(n) if (1..=layouts.len()).contains(&n) => Ok(n - 1),
        _ => {
            let names: Vec<_> = layouts.iter().map(|l| l.name.as_str()).collect();
            bail!(
                "no layout {target:?}: use a name ({}) or a number from 1 to {}",
                names.join(", "),
                layouts.len()
            )
        }
    }
}

fn udev_rule(vid: u16, pid: u16) -> String {
    format!(
        "# /etc/udev/rules.d/50-keyboard-swap.rules\n\
         # Lets the logged-in user access this keyboard's raw HID interface (as VIA needs).\n\
         KERNEL==\"hidraw*\", SUBSYSTEM==\"hidraw\", ATTRS{{idVendor}}==\"{vid:04x}\", \
         ATTRS{{idProduct}}==\"{pid:04x}\", MODE=\"0660\", TAG+=\"uaccess\"\n"
    )
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = match cli.config {
        Some(p) => p,
        None => config::default_path()?,
    };
    let entries = config::load(&config_path)?;
    let layouts = entries.iter().map(Layout::load).collect::<Result<Vec<_>>>()?;

    let (vid, pid) = (layouts[0].vid, layouts[0].pid);
    if let Some(other) = layouts.iter().find(|l| (l.vid, l.pid) != (vid, pid)) {
        bail!(
            "{} is for a different keyboard than {}; use one config per keyboard",
            other.path.display(),
            layouts[0].path.display()
        );
    }
    let keys = layouts[0].keys_per_layer();
    if let Some(other) = layouts.iter().find(|l| l.keys_per_layer() != keys) {
        bail!("{} has a different number of keys per layer", other.path.display());
    }

    if cli.udev_rule {
        print!("{}", udev_rule(vid, pid));
        return Ok(());
    }

    let api = KeyboardApi::new(vid, pid, VIA_USAGE_PAGE, Some(1000)).map_err(|e| {
        anyhow!(
            "could not open keyboard {vid:04x}:{pid:04x}: {e}\n\n\
             If it is plugged in, this is probably a permissions problem. Install a udev rule:\n  \
             keyboard-swap --udev-rule | sudo tee /etc/udev/rules.d/50-keyboard-swap.rules\n  \
             sudo udevadm control --reload && sudo udevadm trigger\n\
             then unplug and replug the keyboard."
        )
    })?;

    let protocol = api.get_protocol_version().map_err(via)?;
    if protocol < MIN_PROTOCOL {
        bail!("keyboard speaks VIA protocol {protocol}, which is too old (need {MIN_PROTOCOL}+)");
    }
    let kc_version = if protocol >= NEW_KEYCODES_PROTOCOL { 6 } else { 5 };
    let board_layers = api.get_layer_count().map_err(via)? as usize;
    let macro_count = api.get_macro_count().map_err(via)? as usize;

    let mut encoded = Vec::with_capacity(layouts.len());
    for l in &layouts {
        if l.layers.len() > board_layers {
            bail!(
                "{} has {} layers but the keyboard only has {board_layers}",
                l.path.display(),
                l.layers.len()
            );
        }
        if l.has_encoders {
            eprintln!("note: {} has encoder settings, which are not applied", l.path.display());
        }
        encoded.push(Encoded {
            keymap: l.encode_keymap(kc_version)?,
            macros: l.encode_macros(kc_version, macro_count)?,
        });
    }

    let layers_used = layouts.iter().map(|l| l.layers.len()).max().unwrap_or(0);
    let board = read_board(&api, layers_used, keys, macro_count)?;
    let current = encoded.iter().position(|e| e.matches(&board));

    if cli.list {
        for (i, l) in layouts.iter().enumerate() {
            let mark = if Some(i) == current { "*" } else { " " };
            println!("{mark} {}. {}  ({})", i + 1, l.name, l.path.display());
        }
        if current.is_none() {
            println!("\nThe keyboard currently matches none of these layouts.");
        }
        return Ok(());
    }

    let target = pick_target(cli.target.as_deref(), &layouts, current)?;
    let name = &layouts[target].name;
    let want = &encoded[target];
    let changed_keys = want.keys_differing(&board);
    let changed_macros = want.macros_differ(&board);
    let from = current.map_or("an unrecognised layout", |c| layouts[c].name.as_str());

    if changed_keys == 0 && !changed_macros {
        println!("Already on {name}.");
        return Ok(());
    }
    let summary = format!(
        "{changed_keys} key{} changed{}",
        if changed_keys == 1 { "" } else { "s" },
        if changed_macros { ", macros rewritten" } else { "" }
    );
    if cli.dry_run {
        println!("Would switch from {from} to {name}: {summary}.");
        return Ok(());
    }

    if changed_keys > 0 {
        api.write_raw_matrix(matrix(keys)?, want.keymap.clone()).map_err(via)?;
    }
    if changed_macros {
        let mut bytes = Vec::new();
        for m in &want.macros {
            bytes.extend(m);
            bytes.push(0);
        }
        api.set_macro_bytes(bytes).map_err(via)?;
    }

    let after = read_board(&api, layers_used, keys, macro_count)?;
    if !want.matches(&after) {
        bail!("wrote {name}, but reading the keyboard back doesn't match; try again or use VIA");
    }
    println!("Switched from {from} to {name}: {summary}.");
    Ok(())
}
