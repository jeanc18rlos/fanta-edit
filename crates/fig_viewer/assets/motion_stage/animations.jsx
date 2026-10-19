// animations.jsx: Fanta's implementation of the Stage starter API that Claude
// Design and Claude Motion projects use. It publishes Stage, Sprite, useTime,
// useTimeline, useSprite, Easing, interpolate, animate and clamp on `window`.
// A project can swap in Claude's own starter: Scene.jsx needs only React and
// useTime().
//
// Render mode (`?__render=1`, or `window.__RENDER_MODE = true`) draws the bare
// stage at the window origin without controls, and every page exposes
// `window.__videoMeta`, `window.__seek(seconds)` and `window.__ready` so video
// exporters can step through frames deterministically.

const clamp = (value, min, max) => Math.max(min, Math.min(max, value));

const Easing = {
  linear: (t) => t,
  easeInQuad: (t) => t * t,
  easeOutQuad: (t) => t * (2 - t),
  easeInOutQuad: (t) => (t < 0.5 ? 2 * t * t : -1 + (4 - 2 * t) * t),
  easeInCubic: (t) => t * t * t,
  easeOutCubic: (t) => 1 - Math.pow(1 - t, 3),
  easeInOutCubic: (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2),
  easeInQuart: (t) => t * t * t * t,
  easeOutQuart: (t) => 1 - Math.pow(1 - t, 4),
  easeInOutQuart: (t) => (t < 0.5 ? 8 * t * t * t * t : 1 - Math.pow(-2 * t + 2, 4) / 2),
  easeInExpo: (t) => (t === 0 ? 0 : Math.pow(2, 10 * t - 10)),
  easeOutExpo: (t) => (t === 1 ? 1 : 1 - Math.pow(2, -10 * t)),
  easeInOutExpo: (t) => {
    if (t === 0 || t === 1) return t;
    return t < 0.5 ? Math.pow(2, 20 * t - 10) / 2 : (2 - Math.pow(2, -20 * t + 10)) / 2;
  },
  easeInSine: (t) => 1 - Math.cos((t * Math.PI) / 2),
  easeOutSine: (t) => Math.sin((t * Math.PI) / 2),
  easeInOutSine: (t) => -(Math.cos(Math.PI * t) - 1) / 2,
  easeInBack: (t) => {
    const overshoot = 1.70158;
    return (overshoot + 1) * t * t * t - overshoot * t * t;
  },
  easeOutBack: (t) => {
    const overshoot = 1.70158;
    return 1 + (overshoot + 1) * Math.pow(t - 1, 3) + overshoot * Math.pow(t - 1, 2);
  },
  easeInOutBack: (t) => {
    const overshoot = 1.70158 * 1.525;
    return t < 0.5
      ? (Math.pow(2 * t, 2) * ((overshoot + 1) * 2 * t - overshoot)) / 2
      : (Math.pow(2 * t - 2, 2) * ((overshoot + 1) * (t * 2 - 2) + overshoot) + 2) / 2;
  },
  easeOutElastic: (t) => {
    if (t === 0 || t === 1) return t;
    return Math.pow(2, -10 * t) * Math.sin((t * 10 - 0.75) * ((2 * Math.PI) / 3)) + 1;
  },
};

// interpolate([0, 0.5, 1], [0, 100, 50], ease) returns t => value. `ease` is one
// easing or one per segment.
function interpolate(input, output, ease = Easing.linear) {
  return (t) => {
    const last = input.length - 1;
    if (last < 0) return undefined;
    if (t <= input[0]) return output[0];
    if (t >= input[last]) return output[last];
    for (let index = 0; index < last; index += 1) {
      if (t >= input[index] && t <= input[index + 1]) {
        const span = input[index + 1] - input[index];
        const local = span === 0 ? 0 : (t - input[index]) / span;
        const easing = Array.isArray(ease) ? ease[index] || Easing.linear : ease;
        return output[index] + (output[index + 1] - output[index]) * easing(local);
      }
    }
    return output[last];
  };
}

// animate({ from, to, start, end, ease }) returns t => value for one tween.
function animate({ from = 0, to = 1, start = 0, end = 1, ease = Easing.easeInOutCubic }) {
  return (t) => {
    if (t <= start) return from;
    if (t >= end) return to;
    return from + (to - from) * ease((t - start) / (end - start));
  };
}

const TimelineContext = React.createContext({ time: 0, duration: 10, playing: false });
const useTimeline = () => React.useContext(TimelineContext);
const useTime = () => useTimeline().time;

const SpriteContext = React.createContext({ localTime: 0, progress: 0, duration: 0, visible: true });
const useSprite = () => React.useContext(SpriteContext);

// <Sprite start end> renders its children only while the playhead is inside
// [start, end]; children may be a function of { localTime, progress, duration }.
function Sprite({ start = 0, end = Infinity, keepMounted = false, children }) {
  const { time } = useTimeline();
  const visible = time >= start && time <= end;
  if (!visible && !keepMounted) return null;
  const duration = end - start;
  const localTime = Math.max(0, time - start);
  const progress = duration > 0 && Number.isFinite(duration) ? clamp(localTime / duration, 0, 1) : 0;
  const value = { localTime, progress, duration, visible };
  return (
    <SpriteContext.Provider value={value}>
      {typeof children === 'function' ? children(value) : children}
    </SpriteContext.Provider>
  );
}

function renderModeRequested() {
  try {
    return new URLSearchParams(window.location.search).has('__render') || window.__RENDER_MODE === true;
  } catch (error) {
    return false;
  }
}

function formatSeconds(seconds) {
  return seconds.toFixed(2);
}

function PlaybackBar({ time, duration, playing, onToggle, onSeek }) {
  return (
    <div
      data-export-hide
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '8px 16px',
        color: '#f6f4ef',
        fontSize: 12,
        fontFamily: 'system-ui, sans-serif',
      }}
    >
      <button
        type="button"
        onClick={onToggle}
        style={{
          minWidth: 64,
          padding: '4px 10px',
          borderRadius: 6,
          border: '1px solid rgba(255,255,255,0.2)',
          background: 'rgba(255,255,255,0.08)',
          color: 'inherit',
          cursor: 'pointer',
        }}
      >
        {playing ? 'Pause' : 'Play'}
      </button>
      <input
        type="range"
        min={0}
        max={duration}
        step={0.001}
        value={time}
        onChange={(event) => onSeek(Number(event.target.value))}
        style={{ flex: 1 }}
        aria-label="Playhead"
      />
      <span style={{ fontVariantNumeric: 'tabular-nums' }}>
        {formatSeconds(time)} / {formatSeconds(duration)} s
      </span>
    </div>
  );
}

// <Stage width height duration fps background loop autoplay> owns the playhead.
// Everything inside reads it through useTime(), so each frame is a pure
// function of time.
function Stage({
  width = 1280,
  height = 720,
  duration = 10,
  background = '#f6f4ef',
  fps = 60,
  loop = true,
  autoplay = true,
  children,
}) {
  const renderMode = React.useMemo(renderModeRequested, []);
  const [time, setTime] = React.useState(0);
  const [playing, setPlaying] = React.useState(autoplay && !renderMode);
  const [scale, setScale] = React.useState(1);
  const viewportRef = React.useRef(null);

  React.useEffect(() => {
    window.__videoMeta = { width, height, duration, fps };
    window.__seek = (seconds) => setTime(clamp(Number(seconds) || 0, 0, duration));
    window.__ready = true;
  }, [width, height, duration, fps]);

  React.useEffect(() => {
    if (!playing) return undefined;
    let previous = null;
    let request = 0;
    const step = (now) => {
      if (previous !== null) {
        const elapsed = (now - previous) / 1000;
        setTime((current) => {
          const next = current + elapsed;
          if (next < duration) return next;
          if (loop && duration > 0) return next % duration;
          setPlaying(false);
          return duration;
        });
      }
      previous = now;
      request = requestAnimationFrame(step);
    };
    request = requestAnimationFrame(step);
    return () => cancelAnimationFrame(request);
  }, [playing, duration, loop]);

  React.useEffect(() => {
    if (renderMode) return undefined;
    const measure = () => {
      const element = viewportRef.current;
      if (!element) return;
      setScale(Math.max(0.05, Math.min(element.clientWidth / width, element.clientHeight / height)));
    };
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, [renderMode, width, height]);

  React.useEffect(() => {
    if (renderMode) return undefined;
    const onKey = (event) => {
      const tag = event.target && event.target.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      if (event.code === 'Space') {
        event.preventDefault();
        setPlaying((value) => !value);
      } else if (event.code === 'ArrowLeft') {
        setTime((value) => clamp(value - (event.shiftKey ? 1 : 0.1), 0, duration));
      } else if (event.code === 'ArrowRight') {
        setTime((value) => clamp(value + (event.shiftKey ? 1 : 0.1), 0, duration));
      } else if (event.key === '0' || event.code === 'Home') {
        setTime(0);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [renderMode, duration]);

  const context = React.useMemo(
    () => ({ time, duration, playing, setTime, setPlaying }),
    [time, duration, playing],
  );
  const canvas = (
    <div
      style={{
        position: renderMode ? 'absolute' : 'relative',
        left: 0,
        top: 0,
        width,
        height,
        background,
        overflow: 'hidden',
      }}
    >
      <TimelineContext.Provider value={context}>{children}</TimelineContext.Provider>
    </div>
  );

  if (renderMode) {
    return (
      <div style={{ position: 'fixed', inset: 0, background }}>
        <style>{'[data-export-hide]{display:none !important}'}</style>
        {canvas}
      </div>
    );
  }

  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'flex',
        flexDirection: 'column',
        background: '#0a0a0a',
      }}
    >
      <div
        ref={viewportRef}
        style={{
          flex: 1,
          minHeight: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          overflow: 'hidden',
        }}
      >
        <div style={{ width: width * scale, height: height * scale, flexShrink: 0 }}>
          <div style={{ width, height, transform: `scale(${scale})`, transformOrigin: '0 0' }}>{canvas}</div>
        </div>
      </div>
      <PlaybackBar
        time={time}
        duration={duration}
        playing={playing}
        onToggle={() => {
          if (!playing && time >= duration) setTime(0);
          setPlaying((value) => !value);
        }}
        onSeek={(seconds) => setTime(clamp(seconds, 0, duration))}
      />
    </div>
  );
}

Object.assign(window, {
  Easing,
  interpolate,
  animate,
  clamp,
  TimelineContext,
  useTime,
  useTimeline,
  Sprite,
  SpriteContext,
  useSprite,
  Stage,
  PlaybackBar,
});
