//! The look of the account mails: the fonts and the colors of the light theme of the web app, in
//! inline styles, and a style sheet with the colors of its dark theme for a client in dark mode.

/// The font of the texts. A client with no Funnel Sans uses Helvetica or Arial.
pub(super) const SANS: &str = "'Funnel Sans', Helvetica, Arial, sans-serif";

/// The font of the title.
pub(super) const DISPLAY: &str = "'Funnel Display', 'Funnel Sans', Helvetica, Arial, sans-serif";

/// The font of a code value and of the copy of a link.
pub(super) const MONO: &str = "'JetBrains Mono', ui-monospace, Menlo, Consolas, monospace";

/// The color around the card.
pub(super) const CANVAS: &str = "#EBEEE2";

/// The color of the card.
pub(super) const CARD: &str = "#F4F7EB";

/// The color of the borders and of the lines.
pub(super) const BORDER: &str = "#E1E3D9";

/// The color of the band at the bottom of the card, and of a neutral badge.
pub(super) const BAND: &str = "#E8EAE0";

/// The color of a title and of a strong text.
pub(super) const FOREGROUND: &str = "#171715";

/// The color of a text.
pub(super) const MUTED: &str = "#686965";

/// The color of the button and of a link.
pub(super) const PRIMARY: &str = "#447857";

/// The color of a text on the button.
pub(super) const ON_PRIMARY: &str = "#F4F7EB";

/// The color of a warning badge, in the two themes.
pub(super) const WARNING: &str = "#ECA851";

/// The style sheet: narrower paddings on a phone, and the colors of the dark theme of the web
/// app. A client with no style sheet keeps the inline styles of the light theme.
pub(super) const SHEET: &str = "
:root { color-scheme: light dark; supported-color-schemes: light dark; }
@media (max-width: 480px) {
  .pad { padding-left: 20px !important; padding-right: 20px !important; }
}
@media (prefers-color-scheme: dark) {
  .canvas { background-color: #0F0F0E !important; }
  .card { background-color: #1E1E1C !important; border-color: #41413F !important; }
  .band { background-color: #2F2F2D !important; }
  .line { border-color: #41413F !important; }
  .fg { color: #F4F7EC !important; }
  .muted { color: #B4B5B1 !important; }
  .link { color: #5D9972 !important; }
  .button { background-color: #5D9972 !important; color: #171715 !important; }
  .outline { border-color: #41413F !important; }
  .neutral { background-color: #2F2F2D !important; color: #F4F7EC !important; }
  .brand { background-color: #5D9972 !important; color: #171715 !important; }
  .logo-light { display: none !important; }
  .logo-dark { display: block !important; max-height: none !important; }
}
";
