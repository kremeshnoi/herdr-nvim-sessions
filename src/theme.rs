use std::env;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::herdr::home_dir;

const PALETTES: &str = include_str!("../palettes.json");
const DEFAULT_THEME: &str = "catppuccin";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Reset,
    Ansi(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    pub fn fg(self) -> String {
        match self {
            Color::Reset => String::new(),
            Color::Ansi(code) => format!("\x1b[{code}m"),
            Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
        }
    }

    pub fn bg(self) -> String {
        match self {
            Color::Reset => String::new(),
            Color::Ansi(code) => format!("\x1b[{}m", code + 10),
            Color::Rgb(r, g, b) => format!("\x1b[48;2;{r};{g};{b}m"),
        }
    }

    pub fn is_reset(self) -> bool {
        self == Color::Reset
    }

    pub fn parse(value: &str) -> Option<Color> {
        let value = value.trim().to_lowercase();
        if matches!(value.as_str(), "reset" | "default" | "none" | "transparent") {
            return Some(Color::Reset);
        }
        if let Some(hex) = value.strip_prefix('#') {
            let digits: String = match hex.len() {
                3 => hex.chars().flat_map(|digit| [digit, digit]).collect(),
                6 => hex.to_owned(),
                _ => return None,
            };
            let channel = |index: usize| u8::from_str_radix(&digits[index..index + 2], 16).ok();
            return Some(Color::Rgb(channel(0)?, channel(2)?, channel(4)?));
        }
        if let Some(inner) = value
            .strip_prefix("rgb(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            let channels: Vec<u8> = inner
                .split(',')
                .filter_map(|part| part.trim().parse().ok())
                .collect();
            return match channels.as_slice() {
                [r, g, b] => Some(Color::Rgb(*r, *g, *b)),
                _ => None,
            };
        }
        let name: String = value
            .chars()
            .filter(|char| !matches!(char, ' ' | '_' | '-'))
            .collect();
        let code = match name.as_str() {
            "black" => 30,
            "red" => 31,
            "green" => 32,
            "yellow" => 33,
            "blue" => 34,
            "magenta" => 35,
            "cyan" => 36,
            "gray" | "grey" => 37,
            "darkgray" | "darkgrey" => 90,
            "lightred" => 91,
            "lightgreen" => 92,
            "lightyellow" => 93,
            "lightblue" => 94,
            "lightmagenta" => 95,
            "lightcyan" => 96,
            "white" => 97,
            _ => return None,
        };
        Some(Color::Ansi(code))
    }
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub active_row_bg: Color,
    pub selection_bg: Color,
    pub overlay0: Color,
    pub text: Color,
    pub subtext0: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
}

fn config_path() -> PathBuf {
    env::var_os("HERDR_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config/herdr/config.toml"))
}

fn theme_config() -> toml::Table {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|content| content.parse::<toml::Table>().ok())
        .and_then(|config| {
            config
                .get("theme")
                .and_then(|theme| theme.as_table())
                .cloned()
        })
        .unwrap_or_default()
}

impl Palette {
    pub fn load() -> Palette {
        let themes: Value = serde_json::from_str(PALETTES).expect("palettes.json is valid");
        let config = theme_config();
        let requested = config
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or(DEFAULT_THEME)
            .to_lowercase()
            .replace([' ', '_'], "-");
        let theme_name = themes["aliases"][&requested]
            .as_str()
            .unwrap_or(DEFAULT_THEME);
        let defaults = &themes["palettes"][theme_name];
        let custom = config.get("custom").and_then(|custom| custom.as_table());
        let color = |key: &str| {
            custom
                .and_then(|custom| custom.get(key))
                .and_then(|value| value.as_str())
                .and_then(Color::parse)
                .or_else(|| defaults[key].as_str().and_then(Color::parse))
                .unwrap_or(Color::Reset)
        };
        Palette {
            active_row_bg: color("active_row_bg"),
            selection_bg: color("selection_bg"),
            overlay0: color("overlay0"),
            text: color("text"),
            subtext0: color("subtext0"),
            green: color("green"),
            yellow: color("yellow"),
            red: color("red"),
        }
    }
}
