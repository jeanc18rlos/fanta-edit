export {};

declare global {
  type FnxHexColor = `#${string}`;

  interface FnxColor {
    readonly r: number;
    readonly g: number;
    readonly b: number;
    readonly a: number;
  }

  function fnxColor(value: FnxHexColor): FnxColor;

  function fnxElement(
    type: unknown,
    props: Readonly<Record<string, unknown>> | null,
    ...children: readonly unknown[]
  ): JSX.Element;

  type FnxPoint = readonly [number, number];
  type FnxSize = readonly [number, number];
  type FnxTransform = readonly [number, number, number, number, number, number];
  type FnxBlendMode =
    | "normal"
    | "multiply"
    | "screen"
    | "overlay"
    | "darken"
    | "lighten"
    | "color_dodge"
    | "color_burn"
    | "hard_light"
    | "soft_light"
    | "difference"
    | "exclusion"
    | "hue"
    | "saturation"
    | "color"
    | "luminosity";

  interface FnxGradientStop {
    position: number;
    color: FnxColor;
  }

  type FnxGradient =
    | {
        kind: "linear";
        start: FnxPoint;
        end: FnxPoint;
        stops: readonly FnxGradientStop[];
      }
    | {
        kind: "radial";
        center: FnxPoint;
        radius: number;
        handles?: readonly [FnxPoint, FnxPoint];
        stops: readonly FnxGradientStop[];
      }
    | {
        kind: "angular";
        center: FnxPoint;
        start_angle: number;
        stops: readonly FnxGradientStop[];
      }
    | {
        kind: "diamond";
        center: FnxPoint;
        radius: number;
        handles?: readonly [FnxPoint, FnxPoint];
        stops: readonly FnxGradientStop[];
      };

  interface FnxImageAdjust {
    exposure?: number;
    contrast?: number;
    saturation?: number;
    temperature?: number;
    tint?: number;
    highlights?: number;
    shadows?: number;
  }

  type FnxImageFitMode = "fill" | "fit" | "stretch" | "tile";
  type FnxFill =
    | { kind: "solid"; color: FnxColor; blend?: FnxBlendMode }
    | { kind: "gradient"; gradient: FnxGradient; blend?: FnxBlendMode }
    | {
        kind: "image";
        asset: string;
        mode: FnxImageFitMode;
        opacity?: number;
        crop?: readonly [number, number, number, number];
        scale?: number;
        rotation?: number;
        blend?: FnxBlendMode;
        adjust?: FnxImageAdjust;
      };

  interface FnxStroke {
    paint: FnxFill;
    width: number;
    cap?: "butt" | "round" | "square";
    join?: "miter" | "round" | "bevel";
    miter_limit?: number;
    dash?: readonly number[];
    align?: "center" | "inside" | "outside";
    per_side?: readonly [number, number, number, number];
  }

  interface FnxShadow {
    kind?: "drop" | "inner";
    color: FnxColor;
    blur: number;
    spread: number;
    offset: FnxPoint;
    show_behind_node?: boolean;
  }

  interface FnxBlur {
    kind?: "layer" | "background";
    radius: number;
  }

  interface FnxAutoLayout {
    mode: "horizontal" | "vertical";
    spacing?: number;
    counter_spacing?: number;
    counter_auto_spacing?: boolean;
    padding?: readonly [number, number, number, number];
    primary_align?: "start" | "center" | "end" | "space_between" | "space_evenly";
    counter_align?: "start" | "center" | "end" | "stretch" | "baseline";
    primary_sizing?: "fixed" | "hug";
    counter_sizing?: "fixed" | "hug";
    wrap?: boolean;
    flow_reverse?: boolean;
    child_layout?: boolean;
    reverse_z?: boolean;
    min_size?: readonly [number | null, number | null];
    max_size?: readonly [number | null, number | null];
  }

  interface FnxLayoutChild {
    grow?: number;
    absolute?: boolean;
    align_self?: "start" | "center" | "end" | "stretch" | "baseline";
  }

  interface FnxFontVariation {
    axis: string;
    value: number;
  }

  interface FnxTextStyle {
    font_family: string;
    size_px: number;
    weight: number;
    italic: boolean;
    underline?: boolean;
    strikethrough?: boolean;
    color: FnxColor;
    letter_spacing: number;
    line_height: number;
    line_height_auto_percent?: number;
    font_variations?: readonly FnxFontVariation[];
  }

  interface FnxTextStyleRun {
    start: number;
    end: number;
    style: FnxTextStyle;
  }

  type FnxPathSegment =
    | { op: "move"; to: FnxPoint }
    | { op: "line"; to: FnxPoint }
    | { op: "quad"; ctrl: FnxPoint; to: FnxPoint }
    | { op: "cubic"; ctrl1: FnxPoint; ctrl2: FnxPoint; to: FnxPoint }
    | { op: "close" };

  interface FnxPathData {
    segments: readonly FnxPathSegment[];
    fill_rule?: "non-zero" | "even-odd";
    subpath_rules?: readonly ("non-zero" | "even-odd")[];
  }

  interface FnxNodeProps {
    [attribute: string]: unknown;
    children?: JSX.Element | readonly JSX.Element[];
    name?: string;
    opacity?: number;
    blend_mode?: FnxBlendMode;
    x?: number;
    y?: number;
    /**
     * Size sugar, accepted on load: folds into `clip_size` on a Frame and
     * `local_size` on every other element (an explicit canonical field wins).
     * The editor always writes the canonical fields back.
     */
    width?: number;
    height?: number;
    transform?: FnxTransform;
    /**
     * Bit-packed node flags (locked = 1<<0, hidden = 1<<1, isolated blend =
     * 1<<2). There is deliberately no `visible`/`locked` boolean attribute —
     * writing one is a silent no-op the engine ignores.
     */
    flags?: number;
    fills?: readonly FnxFill[];
    strokes?: readonly FnxStroke[];
    effects?: readonly FnxShadow[];
    blurs?: readonly FnxBlur[];
    layout_child?: FnxLayoutChild;
    corner_radius?: number;
    corner_radii?: readonly [number, number, number, number];
    /** iOS-squircle amount, 0..=1. */
    corner_smoothing?: number;
    is_mask?: boolean;
    mask_type?: "alpha" | "luminance";
    /** Behavior while an ancestor frame scrolls. */
    scroll_behavior?: "scrolls" | "fixed" | "sticky";
    /**
     * Variable bindings. Readable map form: keys are property names
     * (optionally `name:index`), values are `$Collection/Name` token paths or
     * bare variable ids. The canonical pair-array form also round-trips.
     */
    bindings?:
      | Readonly<Record<string, string>>
      | ReadonlyArray<readonly [Readonly<Record<string, unknown>>, string]>;
    /** Prototype interactions originating from this node. */
    reactions?: ReadonlyArray<Readonly<Record<string, unknown>>>;
    /** Responsive constraints against the parent frame. */
    constraints?: Readonly<Record<string, unknown>>;
    meta?: Readonly<Record<string, unknown>>;
  }

  interface FnxFrameProps extends FnxNodeProps {
    background?: FnxFill;
    background_fills?: readonly FnxFill[];
    clip_size?: FnxSize;
    clip_content?: boolean;
    /** Deprecated coarse toggle; prefer `scroll_direction`. */
    scrollable?: boolean;
    /** Prototype overflow axes (Figma scroll direction). */
    scroll_direction?: "none" | "horizontal" | "vertical" | "both";
    /** Authored initial scroll offset `[x, y]`. */
    scroll_offset?: readonly [number, number];
    auto_layout?: FnxAutoLayout;
    explicit_modes?: Readonly<Record<string, string>>;
  }

  /**
   * `<Rect>` / `<Ellipse>` — authoring sugar for a `<Vector>` with generated
   * path geometry. `width`/`height` are required; an explicit `path` is an
   * error (use `<Vector>` for real path data). Simple shapes print back in
   * this form.
   */
  interface FnxShapeSugarProps extends FnxNodeProps {
    width: number;
    height: number;
  }

  interface FnxTextProps extends FnxNodeProps {
    content?: string;
    style?: FnxTextStyle;
    style_runs?: readonly FnxTextStyleRun[];
    align?: "left" | "center" | "right" | "justify";
    vertical_align?: "top" | "center" | "bottom";
    auto_resize?: "none" | "width_and_height" | "height";
    local_size?: FnxSize;
    max_lines?: number;
    truncate?: boolean;
    paragraph_spacing?: number;
    paragraph_indent?: number;
  }

  interface FnxVectorProps extends FnxNodeProps {
    path?: FnxPathData;
    local_size?: FnxSize;
  }

  interface FnxMediaProps extends FnxNodeProps {
    asset?: string;
    local_size?: FnxSize;
  }

  interface FnxInstanceProps extends FnxNodeProps {
    /** Component NAME (unambiguous) or its 26-character id. */
    component?: string;
    /**
     * Per-instance changes to master descendants. This is an ARRAY of
     * override entries `{ target_path, target_prop, value }` — machine-owned;
     * prefer editing the master or the instance's `prop_values`.
     */
    overrides?: ReadonlyArray<Readonly<Record<string, unknown>>>;
    /** Typed component-prop assignments, keyed by prop id. */
    prop_values?: Readonly<Record<string, unknown>>;
    /** Baked per-instance render data from import — machine-owned. */
    derived?: ReadonlyArray<Readonly<Record<string, unknown>>>;
    local_size?: FnxSize;
  }

  namespace JSX {
    interface Element {}
    interface ElementChildrenAttribute {
      children: {};
    }
  }
}

export function Frame(props: FnxFrameProps): JSX.Element;
export function Rect(props: FnxShapeSugarProps): JSX.Element;
export function Ellipse(props: FnxShapeSugarProps): JSX.Element;
export function Vector(props: FnxVectorProps): JSX.Element;
export function Text(props: FnxTextProps): JSX.Element;
export function Image(props: FnxMediaProps): JSX.Element;
export function Video(props: FnxMediaProps): JSX.Element;
export function Audio(props: FnxMediaProps): JSX.Element;
export function Model3D(props: FnxMediaProps): JSX.Element;
export function Instance(props: FnxInstanceProps): JSX.Element;
export function Boolean(props: FnxNodeProps): JSX.Element;
export function Embed(props: FnxNodeProps): JSX.Element;
export function NodeGraph(props: FnxNodeProps): JSX.Element;
export function AiArtifact(props: FnxNodeProps): JSX.Element;
