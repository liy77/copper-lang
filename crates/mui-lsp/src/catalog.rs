//! MUI widget + enum catalog — the data behind completion and hover.
//!
//! This mirrors `editors/vscode-mui/src/catalog.ts` (the mocida widget surface)
//! and the runtime/codegen widget dispatch. Kept as plain `const` tables so it's
//! trivial to extend in lock-step with the other two.

/// A widget the language knows about.
pub struct Widget {
    pub name: &'static str,
    pub doc: &'static str,
    /// Named props specific to this widget (common props apply to all).
    pub props: &'static [&'static str],
    /// True when the widget usually takes a positional first arg.
    pub positional: bool,
    /// True when the widget commonly has a `{ children }` block.
    pub container: bool,
}

/// Props shared by every widget (the `UIWidget` envelope).
pub const COMMON_PROPS: &[&str] = &[
    "key",
    "id",
    "size",
    "position",
    "anchor",
    "width",
    "height",
    "visible",
    "zIndex",
    "opacity",
    "rotation",
    // Spacing: uniform / array / per-side / per-axis (padding + margin).
    "padding",
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "paddingX",
    "paddingY",
    "margins",
    "margin",
    "marginTop",
    "marginRight",
    "marginBottom",
    "marginLeft",
    "marginX",
    "marginY",
];

/// The widget table. Order matters only for completion display.
pub const WIDGETS: &[Widget] = &[
    Widget {
        name: "Rectangle",
        doc: "The base widget and base container: a filled, rounded, bordered box that can hold children.",
        props: &["fill", "background", "bg", "color", "radius", "borderWidth", "borderColor", "shadow", "padding", "gap"],
        positional: false,
        container: true,
    },
    Widget {
        name: "Stack",
        doc: "Linear container (vertical/horizontal). Supports a background (drawn via a wrapping panel), cross-axis `align`, and per-side padding.",
        props: &["orientation", "gap", "align", "background", "bg", "radius", "borderColor", "borderWidth", "shadow", "opacity"],
        positional: false,
        container: true,
    },
    Widget {
        name: "Grid",
        doc: "Grid container.",
        props: &["columns", "gap"],
        positional: false,
        container: true,
    },
    Widget {
        name: "GridView",
        doc: "Scrollable grid of fixed cells.",
        props: &["columns", "cellWidth", "cellHeight", "cellSize", "gap"],
        positional: false,
        container: true,
    },
    Widget {
        name: "ListView",
        doc: "Vertical scrolling list; each child is a row of `itemHeight`.",
        props: &["itemHeight", "gap"],
        positional: false,
        container: true,
    },
    Widget {
        name: "Scroll",
        doc: "Scroll viewport.",
        props: &["direction", "gap", "wheelSpeed", "dragScroll"],
        positional: false,
        container: true,
    },
    Widget {
        name: "Text",
        doc: "Styled text label. Positional: the text (supports `${...}` interpolation).",
        props: &["color", "align", "hAlign", "vAlign", "wrap", "weight", "fontStyle", "font", "fontFamily", "selectionColor", "cursor"],
        positional: true,
        container: false,
    },
    Widget {
        name: "Button",
        doc: "Clickable button. Positional: the label.",
        props: &["size", "weight", "fontStyle", "font", "fontFamily", "background", "bg", "textColor", "colors", "radius", "borderWidth", "cursor", "shadow", "enabled", "onClick"],
        positional: true,
        container: false,
    },
    Widget {
        name: "Image",
        doc: "Image widget. Positional (or `source`/`src`): the source path.",
        props: &["source", "src", "fillMode", "tint", "animated"],
        positional: true,
        container: false,
    },
    Widget {
        name: "Checkbox",
        doc: "Two-state checkbox (optionally labeled). Box + check colors, border, radius, animation.",
        props: &["value", "checked", "boxColor", "checkColor", "color", "background", "bg", "borderColor", "borderWidth", "radius", "animMs", "cursor", "onChange", "onClick", "label", "textColor", "labelColor", "labelSize", "weight", "fontStyle", "font"],
        positional: true,
        container: false,
    },
    Widget {
        name: "Switch",
        doc: "Boolean toggle switch (optionally labeled). Track + knob colors, border, animation.",
        props: &["value", "checked", "offColor", "onColor", "color", "knobColor", "borderColor", "borderWidth", "animMs", "cursor", "width", "height", "onChange", "onClick", "label", "textColor", "labelColor", "labelSize", "weight", "fontStyle", "font"],
        positional: true,
        container: false,
    },
    Widget {
        name: "RadioButton",
        doc: "Mutually-exclusive radio (optionally labeled). Ring + dot colors, border, dotScale, animation.",
        props: &[
            "group", "value", "selected", "checked", "color", "boxColor", "dotColor", "borderColor",
            "borderWidth", "dotScale", "animMs", "size", "cursor", "enabled", "onClick", "onChange",
            "label", "textColor", "labelColor", "labelSize", "weight", "fontStyle", "font",
        ],
        positional: true,
        container: false,
    },
    Widget {
        name: "Slider",
        doc: "Draggable value slider. Track/fill/knob colors, track height, knob radius.",
        props: &["min", "max", "value", "trackColor", "fillColor", "color", "knobColor", "trackHeight", "knobRadius", "cursor", "onChange"],
        positional: false,
        container: false,
    },
    Widget {
        name: "ProgressBar",
        doc: "Progress indicator (0..1). Track/fill colors, radius, indeterminate sweep.",
        props: &["value", "indeterminate", "animated", "trackColor", "fillColor", "color", "radius"],
        positional: false,
        container: false,
    },
    Widget {
        name: "Spinner",
        doc: "Loading spinner. Color, stroke thickness, rotation speed.",
        props: &["radius", "color", "thickness", "speed"],
        positional: false,
        container: false,
    },
    Widget {
        name: "TextField",
        doc: "Single-line text input. Full style: fill, text/placeholder/caret/selection colors, border, padding, font, password.",
        props: &["value", "text", "placeholder", "background", "bg", "textColor", "color", "placeholderColor", "caretColor", "selectionColor", "borderColor", "borderColorFocused", "borderWidth", "radius", "padding", "fontSize", "weight", "fontStyle", "font", "fontFamily", "password", "maxLength", "caretBlink", "placeholderAnimated", "cursor", "onChange", "onSubmit"],
        positional: false,
        container: false,
    },
    Widget {
        name: "Input",
        doc: "Single-line text input — alias of `TextField`.",
        props: &["value", "text", "placeholder", "background", "bg", "textColor", "color", "placeholderColor", "caretColor", "selectionColor", "borderColor", "borderColorFocused", "borderWidth", "radius", "padding", "fontSize", "weight", "fontStyle", "font", "fontFamily", "password", "maxLength", "caretBlink", "placeholderAnimated", "cursor", "onChange", "onSubmit"],
        positional: false,
        container: false,
    },
    Widget {
        name: "TextArea",
        doc: "Multi-line text input. Fill, text color, border, padding, line spacing, wrap.",
        props: &["value", "text", "placeholder", "background", "bg", "textColor", "color", "borderColor", "borderWidth", "radius", "padding", "lineSpacing", "wrap", "fontSize", "font", "fontFamily", "maxLength", "cursor", "onChange"],
        positional: false,
        container: false,
    },
];

/// Enum name → member list. Drives `FontStyle.` completion + member validation.
pub const ENUMS: &[(&str, &[&str])] = &[
    (
        "FontStyle",
        &["Normal", "Bold", "Italic", "Underline", "Strikethrough"],
    ),
    (
        "FillMode",
        &[
            "None",
            "Stretch",
            "Scale",
            "Tile",
            "Center",
            "Fit",
            "FitWidth",
            "FitHeight",
            "Cover",
        ],
    ),
    ("TextHAlign", &["Left", "Center", "Right"]),
    ("TextVAlign", &["Top", "Center", "Bottom"]),
    ("WrapMode", &["None", "Word", "Char", "Fit"]),
    (
        "Cursor",
        &[
            "Default",
            "Pointer",
            "Text",
            "Crosshair",
            "Move",
            "NotAllowed",
            "Wait",
            "Progress",
        ],
    ),
];

/// Per-prop one-line docs (shown in hover + completion detail).
pub const PROP_DOCS: &[(&str, &str)] = &[
    ("key", "Stable identity for reconciliation. Required on items rendered inside a `for`."),
    ("id", "Lookup id for the widget (maps to `UIWidget_SetId`)."),
    ("size", "Size in logical pixels — for `Text`/`Button` this is the font size in points."),
    ("position", "Position `(x, y)` in the parent, in logical pixels."),
    ("width", "Explicit width in logical pixels."),
    ("height", "Explicit height in logical pixels."),
    ("visible", "Whether the widget is shown (`true` / `false`)."),
    ("zIndex", "Stacking order; higher draws on top."),
    ("margins", "Outer spacing: `left, top, right, bottom`."),
    ("padding", "Inner spacing applied to children."),
    ("orientation", "Layout axis of a `Stack` (`vertical` / `horizontal`)."),
    ("gap", "Spacing between children, in pixels."),
    ("columns", "Number of columns in the grid."),
    ("direction", "Scroll/flow axis."),
    ("color", "Color (`#rrggbb`, `#rrggbbaa`, or `rgba()`)."),
    ("align", "Horizontal text alignment."),
    ("wrap", "Text wrap mode."),
    ("fontStyle", "Font style flags (`FontStyle.Bold | FontStyle.Italic`)."),
    ("selectionColor", "Highlight color for selected text."),
    ("colors", "Per-state color set for the control."),
    ("background", "Fill color (`#rrggbb`, `#rrggbbaa`, or `rgba()`). Alpha sets opacity. Alias: `bg`."),
    ("bg", "Fill color — short alias for `background`."),
    ("textColor", "Label/text color."),
    ("borderColor", "Border color (painted when `borderWidth` > 0)."),
    ("fill", "Fill color of a `Rectangle`. Alias of `background`/`bg`/`color`."),
    ("radius", "Corner radius in pixels."),
    ("borderWidth", "Border thickness in pixels (0 = no border)."),
    ("shadow", "Drop shadow, CSS-style: `\"<dx> <dy> <blur> [spread] <color>\"`, e.g. `\"0 1px 3px rgba(0,0,0,0.1)\"`."),
    ("opacity", "Alpha multiplier for the whole subtree, `0.0`–`1.0`."),
    ("rotation", "Rotation around the widget centre, in degrees."),
    ("cursor", "Mouse cursor shown on hover (`pointer`, `text`, `default`)."),
    ("anchor", "Anchor against the parent: `center`, `top`, `bottom`, `left`, `right`, or a corner like `topLeft`."),
    ("fillMode", "How the image fills its box."),
    ("tint", "Tint color multiplied over the image."),
    ("value", "The current value (bind a `signal` for two-way binding)."),
    ("checked", "Initial on/off state (alias of `value`)."),
    ("onChange", "Handler called when the value changes: `onChange: { |v| ... }`."),
    ("onClick", "Click handler: `onClick: { ... }`."),
    ("group", "Radio group identity; radios sharing it are mutually exclusive."),
    ("label", "Text shown next to the control (e.g. a `RadioButton` caption)."),
    ("selected", "Whether this radio is the selected one in its group (`true` / `false`)."),
    ("enabled", "Whether the control is interactive (`false` = greyed out)."),
    ("dotColor", "Color of the filled dot inside a selected radio."),
    ("placeholder", "Placeholder text shown when empty."),
    ("min", "Minimum value."),
    ("max", "Maximum value."),
    ("indeterminate", "Show the back-and-forth indeterminate animation."),
    // Typography
    ("weight", "Font weight: `bold` or `normal`."),
    ("font", "Font family name (e.g. \"Arial\"); resolved to a file at runtime."),
    ("fontFamily", "Font family name (alias of `font`)."),
    ("fontSize", "Font size in points."),
    ("hAlign", "Horizontal text alignment (`left`/`center`/`right`)."),
    ("vAlign", "Vertical text alignment (`top`/`center`/`bottom`)."),
    ("labelColor", "Caption text color for a labeled control (alias of `textColor`)."),
    ("labelSize", "Caption font size for a labeled control."),
    // Checkbox / Radio / Switch styling
    ("boxColor", "Box/ring fill color of a checkbox or radio."),
    ("checkColor", "Check-mark color of a checkbox."),
    ("offColor", "Track color of a switch when off."),
    ("onColor", "Track color of a switch when on."),
    ("knobColor", "Knob color of a switch."),
    ("dotScale", "Inner-dot size of a radio, as a fraction of the outer radius (0..1)."),
    ("animMs", "Toggle animation duration in milliseconds (0 = instant)."),
    // Slider / ProgressBar / Spinner
    ("trackColor", "Track (background rail) color."),
    ("fillColor", "Filled-portion color."),
    ("trackHeight", "Slider track height in pixels."),
    ("knobRadius", "Slider knob radius in pixels."),
    ("thickness", "Spinner arc stroke width in pixels."),
    ("speed", "Spinner rotation speed in radians per second."),
    // Text inputs
    ("text", "Initial text contents (alias of `value`)."),
    ("placeholderColor", "Placeholder glyph color (shown when empty)."),
    ("caretColor", "Caret (text cursor) color."),
    ("borderColorFocused", "Border color while the field is focused."),
    ("password", "Render dots instead of the typed characters (`true`/`false`)."),
    ("maxLength", "Maximum length in bytes (-1 = unlimited)."),
    ("caretBlink", "Caret blink half-period in ms (0 = no blink)."),
    ("placeholderAnimated", "Gently pulse the placeholder opacity while empty + unfocused."),
    ("lineSpacing", "TextArea line-height multiplier on the font size."),
    ("onSubmit", "Handler called on Enter in a single-line field: `onSubmit: { |v| ... }`."),
    // Containers
    ("itemHeight", "Row height of a `ListView` in pixels."),
    ("cellWidth", "Cell width of a `GridView` in pixels."),
    ("cellHeight", "Cell height of a `GridView` in pixels."),
    ("cellSize", "Cell width AND height of a `GridView` (shorthand)."),
    ("wheelSpeed", "Scroll wheel speed in pixels per notch."),
    ("dragScroll", "Enable drag-to-pan with the left mouse button."),
    // Image
    ("source", "Image source path (a `mocida://` URI resolves through the bundle)."),
    ("src", "Image source path (alias of `source`)."),
    ("animated", "Play animated frames (GIF), or the indeterminate sweep on a progress bar."),
    // Stack alignment + spacing
    ("align", "Cross-axis alignment of a `Stack`'s children: `start` / `center` / `end` (needs the stack wider/taller than its content, e.g. an explicit `width`)."),
    ("paddingTop", "Inner top padding in pixels."),
    ("paddingRight", "Inner right padding in pixels."),
    ("paddingBottom", "Inner bottom padding in pixels."),
    ("paddingLeft", "Inner left padding in pixels."),
    ("paddingX", "Inner left + right padding in pixels."),
    ("paddingY", "Inner top + bottom padding in pixels."),
    ("margin", "Outer spacing. Number (all sides), `[t,r,b,l]` array, or per-side `marginTop`…"),
    ("marginTop", "Outer top margin in pixels."),
    ("marginRight", "Outer right margin in pixels."),
    ("marginBottom", "Outer bottom margin in pixels."),
    ("marginLeft", "Outer left margin in pixels."),
    ("marginX", "Outer left + right margin in pixels."),
    ("marginY", "Outer top + bottom margin in pixels."),
];

/// Values offered for specific props (helps `orientation:` etc.).
pub const PROP_VALUES: &[(&str, &[&str])] = &[
    ("orientation", &["vertical", "horizontal"]),
    ("direction", &["vertical", "horizontal", "both"]),
    ("align", &["start", "center", "end", "left", "right"]),
    ("hAlign", &["left", "center", "right"]),
    ("vAlign", &["top", "center", "bottom"]),
    ("weight", &["normal", "bold"]),
    ("wrap", &["none", "word", "char", "fit"]),
    (
        "fillMode",
        &[
            "none",
            "stretch",
            "scale",
            "tile",
            "center",
            "fit",
            "fitWidth",
            "fitHeight",
            "cover",
        ],
    ),
    (
        "cursor",
        &["Cursor.Pointer", "Cursor.Text", "Cursor.Default"],
    ),
];

/// MUI-specific keywords (markup + reactivity), beyond the Copper ones.
pub const MUI_KEYWORDS: &[(&str, &str, &str)] = &[
    ("view", "view Name(props) { ... }", "Declares a component. Props are typed parameters; the body is a widget tree."),
    ("App", "App() { name:, width:, height:, mainContent: View }", "App config block: window + bundle identity, and the entry component to mount."),
    ("import", "import { Name } from \"./file.mui\"", "Import component views (.mui/.crm) or logic (.crs/.rs) from another file."),
    ("signal", "signal(initial)", "Creates a reactive cell. Reading it in a binding subscribes; writing updates bound widgets."),
    ("computed", "computed { expr }", "A derived, lazily-cached signal that re-reads when its dependencies change."),
    ("effect", "effect { ... }", "A reactive side effect: runs on mount and whenever a signal it reads changes."),
    ("mut", "mut name = signal(0)", "Declares a mutable reactive binding."),
];

/// App-block field names (inside `App() { ... }`).
pub const APP_FIELDS: &[(&str, &str)] = &[
    ("name", "Application + window name."),
    ("id", "Bundle identifier, e.g. \"net.example.app\"."),
    ("title", "Window title (defaults to `name`)."),
    ("width", "Window width in pixels."),
    ("height", "Window height in pixels."),
    ("background", "Window background color."),
    (
        "mainContent",
        "The component (view) to mount as the app's root.",
    ),
];

// ---- lookups ----

pub fn widget(name: &str) -> Option<&'static Widget> {
    WIDGETS.iter().find(|w| w.name == name)
}

pub fn prop_doc(name: &str) -> Option<&'static str> {
    PROP_DOCS.iter().find(|(p, _)| *p == name).map(|(_, d)| *d)
}

pub fn enum_members(name: &str) -> Option<&'static [&'static str]> {
    ENUMS.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
}

pub fn prop_values(name: &str) -> Option<&'static [&'static str]> {
    PROP_VALUES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| *v)
}
