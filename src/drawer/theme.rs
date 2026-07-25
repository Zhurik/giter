use crate::git::remote::Verdict;
use crate::k8s::context::Env;
use ratatui::style::Color;

/// Every colour the interface uses. Built once at start-up: the palette depends on the
/// cluster, on whether the terminal can do 24-bit colour, and on the colour-blind switch.
pub struct Theme {
    pub bg: Color,
    /// Background of the bars that frame the table.
    pub bar: Color,
    /// Background of the key hint line, a shade below the bars.
    pub keys_bar: Color,
    pub line: Color,
    pub selection: Color,
    pub fg: Color,
    pub fg2: Color,
    pub dim: Color,
    pub faint: Color,
    pub ok: Color,
    pub bad: Color,
    pub warn: Color,
    pub cyan: Color,
    /// Colour of the environment badge and of the context name next to it.
    pub env: Color,
    pub env_fg: Color,
    /// Tint of the whole header, so the cluster is visible out of the corner of an eye.
    pub env_bg: Color,
}

impl Theme {
    pub fn new(env: Option<Env>, colorblind: bool) -> Theme {
        match truecolor() {
            true => Theme::rgb(env, colorblind),
            false => Theme::ansi(env, colorblind),
        }
    }

    pub fn verdict(&self, verdict: Verdict) -> Color {
        match verdict {
            Verdict::InSync => self.ok,
            Verdict::Behind => self.bad,
            Verdict::Failed(_) => self.warn,
            Verdict::Resolving | Verdict::Unknown(_) => self.dim,
        }
    }

    fn rgb(env: Option<Env>, colorblind: bool) -> Theme {
        let (badge, badge_fg, tint) = match env {
            Some(Env::Prod) => (0xf2617a, 0x170d11, 0x241820),
            Some(Env::Stage) => (0xe5a95c, 0x191308, 0x231d13),
            Some(Env::Dev) => (0x7dcfff, 0x0d1620, 0x141d26),
            None => (0x98a1b8, 0x161923, 0x191d28),
        };

        Theme {
            bg: rgb(0x161923),
            bar: rgb(0x191d28),
            keys_bar: rgb(0x12151e),
            line: rgb(0x272d3b),
            selection: rgb(0x232a38),
            fg: rgb(0xc8cedd),
            fg2: rgb(0x98a1b8),
            dim: rgb(0x69738c),
            faint: rgb(0x454e63),
            ok: rgb(match colorblind {
                true => 0x5cb8f0,
                false => 0xa3e05c,
            }),
            bad: rgb(match colorblind {
                true => 0xf0883e,
                false => 0xf2617a,
            }),
            warn: rgb(0xe5a95c),
            cyan: rgb(0x7dcfff),
            env: rgb(badge),
            env_fg: rgb(badge_fg),
            env_bg: rgb(tint),
        }
    }

    /// Fallback for terminals that do not announce 24-bit colour: the same roles, drawn
    /// with the sixteen colours every terminal has, and no background tints to get wrong.
    fn ansi(env: Option<Env>, colorblind: bool) -> Theme {
        let badge = match env {
            Some(Env::Prod) => Color::Red,
            Some(Env::Stage) => Color::Yellow,
            Some(Env::Dev) => Color::Cyan,
            None => Color::Gray,
        };

        Theme {
            bg: Color::Reset,
            bar: Color::Reset,
            keys_bar: Color::Reset,
            line: Color::DarkGray,
            // not DarkGray: dimmed text is DarkGray too, and would vanish on the cursor row
            selection: Color::Blue,
            fg: Color::White,
            fg2: Color::Gray,
            dim: Color::DarkGray,
            faint: Color::DarkGray,
            ok: match colorblind {
                true => Color::LightBlue,
                false => Color::Green,
            },
            bad: match colorblind {
                true => Color::LightYellow,
                false => Color::Red,
            },
            warn: Color::Yellow,
            cyan: Color::Cyan,
            env: badge,
            env_fg: Color::Black,
            env_bg: Color::Reset,
        }
    }
}

fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor") | Ok("24bit")
    )
}
