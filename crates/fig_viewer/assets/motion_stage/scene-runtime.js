// Fanta's keyframe evaluator, ported line for line so this package plays the
// clip exactly as Fanta's Motion timeline does: keyframes hold before the first
// and after the last, each keyframe's easing shapes the segment it starts, Hold
// keeps a value until the next keyframe, and rotation and scale turn about the
// layer's declared pivot. A layer's element matrix maps its rest pose to its
// pose at time t, in the coordinates of the element that contains it.
(function (root) {
  'use strict';

  const scene = FANTA_SCENE;

  function clamp(value, low, high) {
    return Math.min(high, Math.max(low, value));
  }

  function bezierCoordinate(parameter, first, second) {
    const inverse = 1 - parameter;
    return (
      3 * inverse * inverse * parameter * first +
      3 * inverse * parameter * parameter * second +
      parameter * parameter * parameter
    );
  }

  function bezierDerivative(parameter, first, second) {
    const inverse = 1 - parameter;
    return (
      3 * inverse * inverse * first +
      6 * inverse * parameter * (second - first) +
      3 * parameter * parameter * (1 - second)
    );
  }

  function cubicBezier(x1, y1, x2, y2) {
    return function (progress) {
      if (progress <= 0 || progress >= 1) return progress;
      let parameter = progress;
      for (let step = 0; step < 8; step += 1) {
        const error = bezierCoordinate(parameter, x1, x2) - progress;
        if (Math.abs(error) <= 1e-7) return bezierCoordinate(parameter, y1, y2);
        const derivative = bezierDerivative(parameter, x1, x2);
        if (Math.abs(derivative) <= 1e-7) break;
        const next = parameter - error / derivative;
        if (!(next >= 0 && next <= 1)) break;
        parameter = next;
      }
      let low = 0;
      let high = 1;
      for (let step = 0; step < 24; step += 1) {
        parameter = (low + high) * 0.5;
        if (bezierCoordinate(parameter, x1, x2) < progress) low = parameter;
        else high = parameter;
      }
      return bezierCoordinate(parameter, y1, y2);
    };
  }

  // A damped spring normalized to settle exactly at the end of its segment.
  function spring(mass, stiffness, damping, t) {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    if (!(mass > 0) || !(stiffness > 0) || !(damping > 0)) return t;
    if (!Number.isFinite(mass) || !Number.isFinite(stiffness) || !Number.isFinite(damping)) return t;
    const omega = Math.sqrt(stiffness / mass);
    const zeta = damping / (2 * Math.sqrt(stiffness * mass));
    const lnTarget = -Math.log(1e-4);
    const slowRate =
      zeta < 1
        ? zeta * omega
        : omega * Math.max(zeta - Math.sqrt(zeta * zeta - 1), Number.EPSILON);
    if (!Number.isFinite(slowRate) || slowRate <= 0) return t;
    const time = t * (lnTarget / slowRate);
    if (zeta < 1 - 1e-9) {
      const omegaD = omega * Math.sqrt(1 - zeta * zeta);
      const envelope = Math.exp(-zeta * omega * time);
      return (
        1 -
        envelope * (Math.cos(omegaD * time) + ((zeta * omega) / omegaD) * Math.sin(omegaD * time))
      );
    }
    if (zeta <= 1 + 1e-9) {
      return 1 - Math.exp(-omega * time) * (1 + omega * time);
    }
    const discriminant = Math.sqrt(zeta * zeta - 1);
    const slow = -omega * (zeta - discriminant);
    const fast = -omega * (zeta + discriminant);
    return 1 - (fast * Math.exp(slow * time) - slow * Math.exp(fast * time)) / (fast - slow);
  }

  const NAMED_EASINGS = {
    linear: function (progress) {
      return progress;
    },
    easeIn: cubicBezier(0.42, 0, 1, 1),
    easeOut: cubicBezier(0, 0, 0.58, 1),
    easeInOut: cubicBezier(0.42, 0, 0.58, 1),
  };

  function easingFunction(easing) {
    if (typeof easing === 'string') return NAMED_EASINGS[easing] || NAMED_EASINGS.linear;
    if (easing && Array.isArray(easing.cubicBezier)) {
      const [x1, y1, x2, y2] = easing.cubicBezier;
      return cubicBezier(x1, y1, x2, y2);
    }
    if (easing && Array.isArray(easing.spring)) {
      const [mass, stiffness, damping] = easing.spring;
      return function (progress) {
        return spring(mass, stiffness, damping, progress);
      };
    }
    return NAMED_EASINGS.linear;
  }

  function prepareTrack(track) {
    if (!track || !Array.isArray(track.times)) return null;
    return {
      milliseconds: track.times.map(function (time) {
        return Math.round(time * 1000);
      }),
      values: track.values,
      easing: track.easing,
      functions: track.easing.map(function (easing) {
        return easing === 'hold' ? null : easingFunction(easing);
      }),
    };
  }

  function sample(track, milliseconds, fallback) {
    if (!track || track.milliseconds.length === 0) return fallback;
    const times = track.milliseconds;
    if (milliseconds < times[0]) return track.values[0];
    let upper = 0;
    while (upper < times.length && times[upper] <= milliseconds) upper += 1;
    if (upper === times.length) return track.values[times.length - 1];
    const lower = upper - 1;
    const from = track.values[lower];
    if (times[lower] === milliseconds || track.easing[lower] === 'hold') return from;
    const segment = times[upper] - times[lower];
    if (segment === 0) return track.values[upper];
    const progress = track.functions[lower](clamp((milliseconds - times[lower]) / segment, 0, 1));
    const to = track.values[upper];
    if (typeof from === 'number' && typeof to === 'number') return from + (to - from) * progress;
    return from;
  }

  function multiply(left, right) {
    return [
      left[0] * right[0] + left[2] * right[1],
      left[1] * right[0] + left[3] * right[1],
      left[0] * right[2] + left[2] * right[3],
      left[1] * right[2] + left[3] * right[3],
      left[0] * right[4] + left[2] * right[5] + left[4],
      left[1] * right[4] + left[3] * right[5] + left[5],
    ];
  }

  function invert(matrix) {
    const determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if (!determinant || !Number.isFinite(determinant)) return null;
    const a = matrix[3] / determinant;
    const b = -matrix[1] / determinant;
    const c = -matrix[2] / determinant;
    const d = matrix[0] / determinant;
    return [a, b, c, d, -(a * matrix[4] + c * matrix[5]), -(b * matrix[4] + d * matrix[5])];
  }

  function compose(x, y, radians, scaleX, scaleY) {
    const sin = Math.sin(radians);
    const cos = Math.cos(radians);
    return [cos * scaleX, sin * scaleX, -sin * scaleY, cos * scaleY, x, y];
  }

  const TRANSFORM_CHANNELS = ['x', 'y', 'rotation', 'scaleX', 'scaleY'];
  const layers = scene.layers.map(function (layer) {
    const tracks = {};
    Object.keys(layer.tracks || {}).forEach(function (channel) {
      tracks[channel] = prepareTrack(layer.tracks[channel]);
    });
    const animatesTransform = TRANSFORM_CHANNELS.some(function (channel) {
      return tracks[channel] && tracks[channel].milliseconds.length > 0;
    });
    return {
      source: layer,
      tracks: tracks,
      animatesTransform: animatesTransform,
      restInverse: invert(layer.restMatrix),
      frameInverse: invert(layer.frame),
    };
  });

  function localMatrix(layer, milliseconds) {
    const rest = layer.source.rest;
    if (!rest || !layer.animatesTransform) return null;
    const tracks = layer.tracks;
    const x = sample(tracks.x, milliseconds, rest.x);
    const y = sample(tracks.y, milliseconds, rest.y);
    const rotation = (sample(tracks.rotation, milliseconds, rest.rotation) * Math.PI) / 180;
    const scaleX = sample(tracks.scaleX, milliseconds, rest.scaleX);
    const scaleY = sample(tracks.scaleY, milliseconds, rest.scaleY);
    const pivot = layer.source.pivot;
    if (!pivot) return compose(x, y, rotation, scaleX, scaleY);
    const restRotation = (rest.rotation * Math.PI) / 180;
    const deltaRotation = rotation - restRotation;
    const deltaX = scaleX / rest.scaleX;
    const deltaY = scaleY / rest.scaleY;
    if (![deltaRotation, deltaX, deltaY, pivot[0], pivot[1]].every(Number.isFinite)) return null;
    const base = compose(x, y, restRotation, rest.scaleX, rest.scaleY);
    const turn = multiply(
      multiply(multiply([1, 0, 0, 1, pivot[0], pivot[1]], compose(0, 0, deltaRotation, 1, 1)), [
        deltaX,
        0,
        0,
        deltaY,
        0,
        0,
      ]),
      [1, 0, 0, 1, -pivot[0], -pivot[1]],
    );
    return multiply(base, turn);
  }

  function elementMatrix(layer, milliseconds) {
    if (!layer.restInverse || !layer.frameInverse) return null;
    const local = localMatrix(layer, milliseconds);
    if (!local || !local.every(Number.isFinite)) return null;
    return multiply(multiply(multiply(layer.source.frame, local), layer.restInverse), layer.frameInverse);
  }

  const stageDuration = scene.loop === 'boomerang' ? scene.duration * 2 : scene.duration;

  // The clip's own playhead for a Stage time: a boomerang plays back down.
  function clipSeconds(seconds) {
    const time = clamp(Number(seconds) || 0, 0, stageDuration);
    return scene.loop === 'boomerang' && time > scene.duration ? stageDuration - time : time;
  }

  function stateAt(seconds) {
    const milliseconds = Math.round(clipSeconds(seconds) * 1000);
    return layers.map(function (layer) {
      const matrix = elementMatrix(layer, milliseconds);
      const opacity = clamp(Number(sample(layer.tracks.opacity, milliseconds, layer.source.opacity)), 0, 1);
      return {
        matrix: matrix,
        transform: matrix ? 'matrix(' + matrix.join(', ') + ')' : 'none',
        opacity: (Number.isFinite(opacity) ? opacity : 0) * layer.source.inheritedOpacity,
        visible: Boolean(sample(layer.tracks.visible, milliseconds, layer.source.visible)),
      };
    });
  }

  function roots() {
    const result = [];
    scene.layers.forEach(function (layer, index) {
      if (layer.parent === null || layer.parent === undefined) result.push(index);
    });
    return result;
  }

  function childrenOf(parent) {
    const result = [];
    scene.layers.forEach(function (layer, index) {
      if (layer.parent === parent) result.push(index);
    });
    return result;
  }

  root.FantaScene = {
    scene: scene,
    stageDuration: stageDuration,
    clipSeconds: clipSeconds,
    stateAt: stateAt,
    roots: roots,
    childrenOf: childrenOf,
    easing: { cubicBezier: cubicBezier, spring: spring, named: NAMED_EASINGS },
  };
})(typeof window !== 'undefined' ? window : globalThis);
