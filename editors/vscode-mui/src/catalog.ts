// MUI widget + enum catalog. Mirrors the element→mocida mapping in
// mui/SPEC.md §7 and the mocida public headers. Drives autocomplete, hover
// tooltips, and diagnostics.
//
// Keep this in sync with the mocida widget surface; it is intentionally a
// plain data table so it is trivial to extend. Descriptions here are the
// single source of truth for the hover "bubbles".

export interface WidgetDef {
  /** Element name, e.g. "Stack". */
  name: string;
  /** One-line description for completion detail + hover. */
  doc: string;
  /** Common named props for this element. */
  props: string[];
  /** True if the element usually takes a positional first arg (label/source). */
  positional?: boolean;
  /** Describes the positional first arg, when there is one. */
  positionalDoc?: string;
  /** True if the element commonly has a `{ children }` block. */
  container?: boolean;
}

// Props shared by every widget (the UIWidget envelope).
export const COMMON_PROPS = [
  'key',
  'id',
  'size',
  'position',
  'anchor',
  'width',
  'height',
  'visible',
  'zIndex',
  'margins',
  'margin',
  'marginTop',
  'marginRight',
  'marginBottom',
  'marginLeft',
  'marginX',
  'marginY',
  'padding',
  'paddingTop',
  'paddingRight',
  'paddingBottom',
  'paddingLeft',
  'paddingX',
  'paddingY',
  'opacity',
  'rotation',
];

export const WIDGETS: WidgetDef[] = [
  { name: 'Rectangle', doc: 'The base widget and base container: a filled, rounded, bordered box that can hold children.', container: true,
    props: ['fill', 'background', 'bg', 'color', 'radius', 'borderWidth', 'borderColor', 'shadow', 'padding', 'gap', 'opacity', 'rotation'] },
  { name: 'Stack', doc: 'Linear container (vertical/horizontal).', container: true,
    props: ['orientation', 'gap', 'align', 'padding', 'background', 'bg', 'radius', 'borderColor', 'borderWidth', 'shadow', 'opacity'] },
  { name: 'Grid', doc: 'Grid container.', container: true, props: ['columns', 'gap'] },
  { name: 'GridView', doc: 'Scrollable grid of fixed cells.', container: true, props: ['columns', 'cellWidth', 'cellHeight', 'cellSize', 'gap'] },
  { name: 'ListView', doc: 'Vertical scrolling list; each child is a row of `itemHeight`.', container: true, props: ['itemHeight', 'gap'] },
  { name: 'Scroll', doc: 'Scroll viewport.', container: true, props: ['direction', 'gap', 'wheelSpeed', 'dragScroll'] },
  { name: 'Text', doc: 'Styled text label.', positional: true,
    positionalDoc: 'The text to display (a string; supports `${...}` interpolation).',
    props: ['color', 'align', 'hAlign', 'vAlign', 'wrap', 'weight', 'fontStyle', 'font', 'fontFamily', 'selectionColor', 'cursor'] },
  { name: 'Button', doc: 'Clickable button.', positional: true,
    positionalDoc: 'The button label (a string).',
    props: ['size', 'weight', 'fontStyle', 'font', 'fontFamily', 'background', 'bg', 'textColor', 'colors', 'radius', 'borderWidth', 'cursor', 'shadow', 'enabled', 'onClick'] },
  { name: 'Image', doc: 'Image widget.', positional: true,
    positionalDoc: 'The image source path (a string); same as `source:`.',
    props: ['source', 'src', 'fillMode', 'tint', 'animated'] },
  { name: 'Checkbox', doc: 'Two-state checkbox (optionally labeled).', positional: true,
    positionalDoc: 'The caption text (same as `label:`).',
    props: ['value', 'checked', 'boxColor', 'checkColor', 'color', 'background', 'bg', 'borderColor', 'borderWidth', 'radius', 'animMs', 'cursor', 'onChange', 'onClick', 'label', 'textColor', 'labelColor', 'labelSize', 'weight', 'fontStyle', 'font'] },
  { name: 'Switch', doc: 'Boolean toggle switch (optionally labeled).', positional: true,
    positionalDoc: 'The caption text (same as `label:`).',
    props: ['value', 'checked', 'offColor', 'onColor', 'color', 'knobColor', 'borderColor', 'borderWidth', 'animMs', 'cursor', 'width', 'height', 'onChange', 'onClick', 'label', 'textColor', 'labelColor', 'labelSize', 'weight', 'fontStyle', 'font'] },
  { name: 'RadioButton', doc: 'Mutually-exclusive radio (optionally labeled).', positional: true,
    positionalDoc: 'The label text (a string); same as `label:`.',
    props: ['group', 'value', 'selected', 'checked', 'color', 'boxColor', 'dotColor', 'borderColor', 'borderWidth', 'dotScale', 'animMs', 'size', 'cursor', 'enabled', 'onClick', 'onChange', 'label', 'textColor', 'labelColor', 'labelSize', 'weight', 'fontStyle', 'font'] },
  { name: 'Slider', doc: 'Draggable value slider.', props: ['min', 'max', 'value', 'trackColor', 'fillColor', 'color', 'knobColor', 'trackHeight', 'knobRadius', 'cursor', 'onChange'] },
  { name: 'ProgressBar', doc: 'Progress indicator (0..1).', props: ['value', 'indeterminate', 'animated', 'trackColor', 'fillColor', 'color', 'radius'] },
  { name: 'Spinner', doc: 'Loading spinner.', props: ['radius', 'color', 'thickness', 'speed'] },
  { name: 'TextField', doc: 'Single-line text input.', props: ['value', 'text', 'placeholder', 'background', 'bg', 'textColor', 'color', 'placeholderColor', 'caretColor', 'selectionColor', 'borderColor', 'borderColorFocused', 'borderWidth', 'radius', 'padding', 'fontSize', 'weight', 'fontStyle', 'font', 'fontFamily', 'password', 'maxLength', 'caretBlink', 'placeholderAnimated', 'cursor', 'onChange', 'onSubmit'] },
  { name: 'Input', doc: 'Single-line text input — alias of `TextField`.', props: ['value', 'text', 'placeholder', 'background', 'bg', 'textColor', 'color', 'placeholderColor', 'caretColor', 'selectionColor', 'borderColor', 'borderColorFocused', 'borderWidth', 'radius', 'padding', 'fontSize', 'weight', 'fontStyle', 'font', 'fontFamily', 'password', 'maxLength', 'caretBlink', 'placeholderAnimated', 'cursor', 'onChange', 'onSubmit'] },
  { name: 'TextArea', doc: 'Multi-line text input.', props: ['value', 'text', 'placeholder', 'background', 'bg', 'textColor', 'color', 'borderColor', 'borderWidth', 'radius', 'padding', 'lineSpacing', 'wrap', 'fontSize', 'font', 'fontFamily', 'maxLength', 'cursor', 'onChange'] },
  { name: 'Dropdown', doc: 'Dropdown selector.', container: true, props: ['onSelect'] },
  { name: 'Menu', doc: 'Menu list.', container: true, props: ['onSelect'] },
  { name: 'Tooltip', doc: 'Hover tooltip.', container: true, props: ['text'] },
  { name: 'Dialog', doc: 'Modal dialog.', container: true, props: ['open', 'onClose'] },
  { name: 'TabView', doc: 'Tabbed container.', container: true, props: ['tabs'] },
  { name: 'Video', doc: 'Video surface.', positional: true,
    positionalDoc: 'The video source path (a string).', props: ['source'] },
  { name: 'WebView', doc: 'Embedded WebView2.', props: ['url'] },
  { name: 'MouseArea', doc: 'Pointer-event region.', container: true, props: ['onPress', 'onDrag'] },
];

// Enum name → member list. Drives `FontStyle.` completion + member validation.
export const ENUMS: Record<string, string[]> = {
  FontStyle: ['Normal', 'Bold', 'Italic', 'Underline', 'Strikethrough'],
  FillMode: ['None', 'Stretch', 'Scale', 'Tile', 'Center', 'Fit', 'FitWidth', 'FitHeight', 'Cover'],
  TextHAlign: ['Left', 'Center', 'Right'],
  TextVAlign: ['Top', 'Center', 'Bottom'],
  WrapMode: ['None', 'Word', 'Char', 'Fit'],
  Cursor: ['Default', 'Pointer', 'Text', 'Crosshair', 'Move', 'NotAllowed', 'Wait', 'Progress'],
};

// One-line description per enum (shown in hover).
export const ENUM_DOCS: Record<string, string> = {
  FontStyle: 'Text style flags — combine with `|`, e.g. `FontStyle.Bold | FontStyle.Italic`.',
  FillMode: 'How an image fills its box.',
  TextHAlign: 'Horizontal text alignment within the widget bounds.',
  TextVAlign: 'Vertical text alignment within the widget bounds.',
  WrapMode: 'Text wrapping strategy.',
  Cursor: 'Mouse cursor shown while hovering the widget.',
};

// Per-prop descriptions for hover bubbles. Most props mean the same across
// widgets, so a single table covers them; the value type is appended when
// `PROP_VALUES` knows it.
export const PROP_DOCS: Record<string, string> = {
  // UIWidget envelope (common)
  key: 'Stable identity for reconciliation. Required on items rendered inside a `for`.',
  id: 'Lookup id for the widget (maps to `UIWidget_SetId`).',
  size: 'Size in logical pixels — for `Text`/`Button` this is the font size in points.',
  position: 'Position `(x, y)` in the parent, in logical pixels.',
  width: 'Explicit width in logical pixels.',
  height: 'Explicit height in logical pixels.',
  visible: 'Whether the widget is shown (`true` / `false`).',
  zIndex: 'Stacking order; higher draws on top.',
  margins: 'Outer spacing: `left, top, right, bottom`.',
  margin: 'Outer spacing. Number (all sides), `[t,r,b,l]` array, or per-side `marginTop`…',
  marginTop: 'Outer top margin in pixels.',
  marginRight: 'Outer right margin in pixels.',
  marginBottom: 'Outer bottom margin in pixels.',
  marginLeft: 'Outer left margin in pixels.',
  marginX: 'Outer left + right margin in pixels.',
  marginY: 'Outer top + bottom margin in pixels.',
  padding: 'Inner spacing. Number (all sides), `[t,r,b,l]` array, or per-side `paddingTop`…',
  paddingTop: 'Inner top padding in pixels.',
  paddingRight: 'Inner right padding in pixels.',
  paddingBottom: 'Inner bottom padding in pixels.',
  paddingLeft: 'Inner left padding in pixels.',
  paddingX: 'Inner left + right padding in pixels.',
  paddingY: 'Inner top + bottom padding in pixels.',
  // layout
  orientation: 'Layout axis of a `Stack`.',
  gap: 'Spacing between children, in pixels.',
  columns: 'Number of columns in the grid.',
  direction: 'Scroll/flow axis.',
  // text & paint
  color: 'Color (`#rrggbb`, `#rrggbbaa`, or `rgba()`).',
  align: 'Text: horizontal alignment. `Stack`: cross-axis alignment of children (`start`/`center`/`end`).',
  wrap: 'Text wrap mode.',
  fontStyle: 'Font style flags (`FontStyle.Bold | FontStyle.Italic`).',
  selectionColor: 'Highlight color for selected text.',
  colors: 'Per-state color set for the control.',
  background: 'Fill color (`#rrggbb`, `#rrggbbaa`, or `rgba()`). Alpha sets opacity. Used by `Stack`, `Rectangle`, `Button`, etc. Alias: `bg`.',
  bg: 'Fill color — short alias for `background`.',
  textColor: 'Label/text color (`#rrggbb`, `#rrggbbaa`, or `rgba()`).',
  borderColor: 'Border color (painted when `borderWidth` > 0).',
  fill: 'Fill color of a `Rectangle` (`#rrggbb`, `#rrggbbaa`, or `rgba()`). Alias of `background`/`bg`/`color`.',
  radius: 'Corner radius in pixels.',
  borderWidth: 'Border thickness in pixels (0 = no border).',
  shadow: 'Drop shadow, CSS-style: `"<dx> <dy> <blur> [spread] <color>"`, e.g. `"0 1px 3px rgba(0,0,0,0.1)"`. `true` uses a default shadow.',
  opacity: 'Alpha multiplier for the whole subtree, `0.0`–`1.0`.',
  rotation: 'Rotation around the widget centre, in degrees.',
  cursor: 'Mouse cursor shown on hover (`pointer`, `text`, `default`).',
  anchor: 'Anchor against the parent: `center`, `top`, `bottom`, `left`, `right`, or a corner like `topLeft` / `bottomRight`.',
  fillMode: 'How the image fills its box.',
  tint: 'Tint color multiplied over the image.',
  // inputs
  value: 'The current value (bind a `signal` for two-way binding).',
  onChange: 'Handler called when the value changes: `onChange: { |v| ... }`.',
  onClick: 'Click handler: `onClick: { ... }`.',
  placeholder: 'Placeholder text shown when empty.',
  min: 'Minimum value.',
  max: 'Maximum value.',
  indeterminate: 'Show the back-and-forth indeterminate animation.',
  group: 'Radio group identity; radios sharing it are mutually exclusive.',
  label: 'Text shown next to the control (e.g. a `RadioButton` caption).',
  selected: 'Whether this radio is the selected one in its group (`true` / `false`).',
  enabled: 'Whether the control is interactive (`false` = greyed out / disabled).',
  dotColor: 'Color of the filled dot inside a selected radio.',
  onSelect: 'Handler called when an item is selected.',
  tabs: 'The tab definitions.',
  open: 'Whether the dialog is open (bind a `signal`).',
  onClose: 'Handler called when the dialog is dismissed.',
  source: 'Media source path.',
  url: 'The URL to load.',
  onPress: 'Pointer-press handler.',
  onDrag: 'Drag handler.',
  text: 'Text content — a tooltip’s text, or a text input’s initial contents (alias of `value`).',
  // typography
  weight: 'Font weight: `bold` or `normal`.',
  font: 'Font family name (e.g. "Arial"); resolved to a file at runtime.',
  fontFamily: 'Font family name (alias of `font`).',
  fontSize: 'Font size in points.',
  hAlign: 'Horizontal text alignment (`left`/`center`/`right`).',
  vAlign: 'Vertical text alignment (`top`/`center`/`bottom`).',
  labelColor: 'Caption text color for a labeled control (alias of `textColor`).',
  labelSize: 'Caption font size for a labeled control.',
  // checkbox / radio / switch
  boxColor: 'Box/ring fill color of a checkbox or radio.',
  checkColor: 'Check-mark color of a checkbox.',
  offColor: 'Track color of a switch when off.',
  onColor: 'Track color of a switch when on.',
  knobColor: 'Knob color of a switch.',
  dotScale: 'Inner-dot size of a radio, as a fraction of the outer radius (0..1).',
  animMs: 'Toggle animation duration in milliseconds (0 = instant).',
  // slider / progress / spinner
  trackColor: 'Track (background rail) color.',
  fillColor: 'Filled-portion color.',
  trackHeight: 'Slider track height in pixels.',
  knobRadius: 'Slider knob radius in pixels.',
  thickness: 'Spinner arc stroke width in pixels.',
  speed: 'Spinner rotation speed in radians per second.',
  // text inputs
  placeholderColor: 'Placeholder glyph color (shown when empty).',
  caretColor: 'Caret (text cursor) color.',
  borderColorFocused: 'Border color while the field is focused.',
  password: 'Render dots instead of the typed characters (`true`/`false`).',
  maxLength: 'Maximum length in bytes (-1 = unlimited).',
  caretBlink: 'Caret blink half-period in ms (0 = no blink).',
  placeholderAnimated: 'Gently pulse the placeholder opacity while empty + unfocused.',
  lineSpacing: 'TextArea line-height multiplier on the font size.',
  onSubmit: 'Handler called on Enter in a single-line field: `onSubmit: { |v| ... }`.',
  // containers
  itemHeight: 'Row height of a `ListView` in pixels.',
  cellWidth: 'Cell width of a `GridView` in pixels.',
  cellHeight: 'Cell height of a `GridView` in pixels.',
  cellSize: 'Cell width AND height of a `GridView` (shorthand).',
  wheelSpeed: 'Scroll wheel speed in pixels per notch.',
  dragScroll: 'Enable drag-to-pan with the left mouse button.',
  // image
  src: 'Image source path (alias of `source`).',
  animated: 'Play animated frames (GIF), or the indeterminate sweep on a progress bar.',
};

// Values often used for specific props (helps `orientation:` etc. + hover).
export const PROP_VALUES: Record<string, string[]> = {
  orientation: ['vertical', 'horizontal'],
  direction: ['vertical', 'horizontal', 'both'],
  align: ['start', 'center', 'end', 'left', 'right'],
  hAlign: ['left', 'center', 'right'],
  vAlign: ['top', 'center', 'bottom'],
  weight: ['normal', 'bold'],
  fillMode: ENUMS.FillMode.map((m) => `FillMode.${m}`),
  fontStyle: ENUMS.FontStyle.map((m) => `FontStyle.${m}`),
  cursor: ENUMS.Cursor.map((m) => `Cursor.${m}`),
  wrap: ENUMS.WrapMode.map((m) => `WrapMode.${m}`),
};

// A keyword/type description: `label` + a signature `detail` + a longer `doc`.
export interface KeywordDef {
  label: string;
  detail: string;
  doc: string;
}

// MUI/Mocida-specific words that are NOT Copper keywords — the markup + the
// reactivity primitives. Copper's own keywords (func, mut, if, struct, impl,
// import, …) and primitive types are loaded from the SHARED copper-lexicon.json
// at activation (see loadCopperLexicon) so their text is identical to the
// Copper extension and can never drift.
export const MUI_KEYWORDS: KeywordDef[] = [
  { label: 'view', detail: 'view Name(props) { ... }',
    doc: 'Declares a component. Props are typed parameters; the body is a widget tree.' },
  { label: 'signal', detail: 'signal(initial)',
    doc: 'Creates a reactive cell (a mocida `Signal`). Reading it in a binding subscribes that spot; writing it updates only the bound widgets.' },
  { label: 'computed', detail: 'computed { expr }',
    doc: 'A derived, lazily-cached signal that re-reads when its dependencies change.' },
  { label: 'effect', detail: 'effect { ... }',
    doc: 'A reactive side effect: runs once on mount and again whenever a signal it reads changes.' },
];

// Filled in by loadCopperLexicon() at activation. Until then they're empty, so
// hover/completion simply fall back to the MUI-only set.
export let COPPER_KEYWORDS: KeywordDef[] = [];
export let COPPER_TYPES: KeywordDef[] = [];

/**
 * Combined keyword list (MUI extras + Copper keywords) used by completion.
 * MUI entries win on label collisions (none today), so the Copper docs apply
 * to func/if/for/match/import/mut/… exactly as in the Copper extension.
 */
export function allKeywords(): KeywordDef[] {
  const seen = new Set(MUI_KEYWORDS.map((k) => k.label));
  return [...MUI_KEYWORDS, ...COPPER_KEYWORDS.filter((k) => !seen.has(k.label))];
}

/**
 * Load the shared Copper lexicon (keywords + primitive types) and populate
 * COPPER_KEYWORDS / COPPER_TYPES. `raw` is the parsed JSON. Falls back to the
 * embedded copy when the canonical file isn't available (e.g. packaged vsix).
 */
export function loadCopperLexicon(raw?: unknown): void {
  const data = (raw ?? EMBEDDED_COPPER_LEXICON) as {
    keywords?: KeywordDef[];
    types?: KeywordDef[];
  };
  COPPER_KEYWORDS = (data.keywords ?? []).map((k) => ({
    label: (k as any).name ?? k.label,
    detail: k.detail,
    doc: k.doc,
  }));
  COPPER_TYPES = (data.types ?? []).map((t) => ({
    label: (t as any).name ?? t.label,
    detail: t.detail,
    doc: t.doc,
  }));
}

// Embedded mirror of editors/copper-lexicon.json — the fallback used when the
// canonical file can't be read. Keep IN SYNC with that file (and with
// copper-lsp's builtin_info). The canonical file is the source of truth; this
// copy only exists so a standalone-packaged extension still has the text.
const EMBEDDED_COPPER_LEXICON = {
  keywords: [
    { name: 'func', detail: 'func RetType name(params)', doc: 'Declares a function.' },
    { name: 'mut', detail: 'mut name = value', doc: 'Declares a mutable variable.' },
    { name: 'let', detail: 'let name = value', doc: 'Declares an immutable variable.' },
    { name: 'if', detail: 'if cond { ... } else { ... }', doc: 'Conditional branch.' },
    { name: 'else', detail: 'else { ... }', doc: 'Else branch.' },
    { name: 'match', detail: 'match x { pattern => result }', doc: 'Pattern matching.' },
    { name: 'for', detail: 'for x in iter { ... }', doc: 'Iterates over a sequence.' },
    { name: 'while', detail: 'while cond { ... }', doc: 'Loops while a condition holds.' },
    { name: 'loop', detail: 'loop { ... }', doc: 'Infinite loop; exit with break.' },
    { name: 'break', detail: 'break', doc: 'Exit loop.' },
    { name: 'continue', detail: 'continue', doc: 'Skip iteration.' },
    { name: 'return', detail: 'return value', doc: 'Return from function.' },
    { name: 'struct', detail: 'struct Name { field: Type }', doc: 'Defines a structure.' },
    { name: 'class', detail: 'class Name { ... }', doc: 'Defines a class with methods.' },
    { name: 'impl', detail: 'impl Type { ... }', doc: 'Implementation block for a type.' },
    { name: 'trait', detail: 'trait Name { ... }', doc: 'Defines a trait (shared behavior).' },
    { name: 'enum', detail: 'enum Name { Variant, ... }', doc: 'Defines an enumeration type.' },
    { name: 'import', detail: 'import { items } from module', doc: 'Imports items from a module.' },
    { name: 'from', detail: 'import x from module', doc: 'Module path in `import`.' },
    { name: 'pub', detail: 'pub ...', doc: 'Public visibility.' },
    { name: 'true', detail: 'true', doc: 'Boolean literal.' },
    { name: 'false', detail: 'false', doc: 'Boolean literal.' },
  ],
  types: [
    { name: 'int', detail: 'int', doc: '64-bit signed integer (alias for i64).' },
    { name: 'float', detail: 'float', doc: '64-bit floating point (alias for f64).' },
    { name: 'string', detail: 'string', doc: 'Owned UTF-8 string (alias for String).' },
    { name: 'bool', detail: 'bool', doc: 'Boolean (true or false).' },
  ],
};

// ---- lookup helpers (shared by completion, hover, diagnostics) ----

export function widgetByName(name: string): WidgetDef | undefined {
  return WIDGETS.find((w) => w.name === name);
}

export function isKnownProp(widget: WidgetDef | undefined, name: string): boolean {
  if (COMMON_PROPS.includes(name)) return true;
  return !!widget && widget.props.includes(name);
}

/** Look up any keyword (MUI or Copper) or primitive type by exact label. */
export function keywordByLabel(label: string): KeywordDef | undefined {
  return (
    MUI_KEYWORDS.find((k) => k.label === label) ??
    COPPER_KEYWORDS.find((k) => k.label === label) ??
    COPPER_TYPES.find((k) => k.label === label)
  );
}
