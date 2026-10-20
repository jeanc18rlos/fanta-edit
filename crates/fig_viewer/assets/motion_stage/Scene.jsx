// Scene.jsx: the exported Fanta animation as a Stage scene. It reads only the
// Stage playhead through useTime() and draws Fanta-rendered bitmaps: the frame
// without its animated layers, then each animated layer moved by scene.js.
// Nested layers sit inside their animated parent's element, so they inherit
// its motion and opacity as they do in Fanta.

function FantaClip({ path, width, height, children }) {
  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        top: 0,
        width,
        height,
        clipPath: `path('${path}')`,
        pointerEvents: 'none',
      }}
    >
      {children}
    </div>
  );
}

function FantaImage({ image, name }) {
  const [x, y, width, height] = image.box;
  return (
    <img
      src={image.src}
      alt={name}
      draggable={false}
      style={{ position: 'absolute', left: x, top: y, width, height, maxWidth: 'none' }}
    />
  );
}

function FantaLayer({ index, states }) {
  const fanta = window.FantaScene;
  const { width, height } = fanta.scene;
  const layer = fanta.scene.layers[index];
  const state = states[index];
  let element = (
    <div
      style={{
        position: 'absolute',
        left: 0,
        top: 0,
        width,
        height,
        transformOrigin: '0 0',
        transform: state.transform,
        opacity: state.opacity,
        display: state.visible ? 'block' : 'none',
        mixBlendMode: layer.blend || 'normal',
        pointerEvents: 'none',
      }}
    >
      {layer.image && <FantaImage image={layer.image} name={layer.name} />}
      {fanta.childrenOf(index).map((child) => (
        <FantaLayer key={child} index={child} states={states} />
      ))}
    </div>
  );
  for (let clip = layer.clips.length - 1; clip >= 0; clip -= 1) {
    element = (
      <FantaClip path={layer.clips[clip]} width={width} height={height}>
        {element}
      </FantaClip>
    );
  }
  return element;
}

function Scene() {
  const time = useTime();
  const fanta = window.FantaScene;
  const { width, height, background } = fanta.scene;
  const states = fanta.stateAt(time);
  return (
    <div style={{ position: 'absolute', left: 0, top: 0, width, height, overflow: 'hidden' }}>
      <FantaImage image={background} name="" />
      {fanta.roots().map((index) => (
        <FantaLayer key={index} index={index} states={states} />
      ))}
    </div>
  );
}

Object.assign(window, { Scene });
