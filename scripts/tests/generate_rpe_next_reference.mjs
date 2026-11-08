// Generate numerical reference data by executing an unmodified rpe-next checkout.
// Usage: node scripts/tests/generate_rpe_next_reference.mjs /path/to/rpe-next OUTPUT.json
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
const root = resolve(process.argv[2]);
const load = file => import(pathToFileURL(resolve(root, file)));
const { NoiseAreaRuntime, validateNoiseAreas } = await load('src/core/noise-areas.mjs');
const { TempoMap } = await load('src/core/tempo.mjs');
const { previewNoiseGeometry, previewPositionScale } = await load('src/core/editor-display.mjs');
const beat = x => [Math.floor(x), Math.round((x - Math.floor(x)) * 1000000), 1000000];
const bpm = [{ bpm: 120, startTime: beat(0) }, { bpm: 60, startTime: beat(4) }, { bpm: 180, startTime: beat(9) }];
const event = (start, end, a = 0, b = 4, extra = {}) => ({ start, end, startTime: beat(a), endTime: beat(b), easingType: 1, ...extra });
const activation = (a, b, readyFlash = 0.5) => ({ startTime: beat(a), endTime: beat(b), readyFlash });
const area = extra => ({ bottomLeft: { x: -250, y: -150 }, topRight: { x: 250, y: 150 }, appearTime: beat(0), disappearTime: beat(16),
    activeIntervals: [activation(4, 7), activation(9, 12)], isInvert: false, moveXEvents: [], moveYEvents: [], rotateEvents: [], scaleXEvents: [], scaleYEvents: [], ...extra });
const cases = [];
const add = (name, definition, times = [-0.1, 0, 0.1, 0.5, 1, 1.5, 1.75, 2, 2.25, 3, 3.5, 4.5, 6, 7, 8]) => {
    validateNoiseAreas([definition]);
    const tempo = new TempoMap(bpm), runtime = new NoiseAreaRuntime(definition, tempo);
    const samples = [];
    for (const aspect of [1.5, 16 / 9, 9 / 16, 1]) for (const time of times) {
        const sample = runtime.sample(time);
        const projected = previewNoiseGeometry(sample, previewPositionScale(aspect));
        const unit = 10 * aspect / Math.min(1350, 900 * aspect);
        samples.push({ time, aspect, state: sample.state, mix: sample.activationMix,
            center: [projected.center.x * unit, projected.center.y * unit],
            points: projected.points.map(p => [p.x * unit, p.y * unit]) });
    }
    cases.push({ name, bpm, area: definition, samples });
};
add('lifetime-clips-and-adjacent-intervals', area({ appearTime: beat(2), disappearTime: beat(12),
    activeIntervals: [activation(-2, 3), activation(5, 7, false), activation(7, 8, true), activation(9, 20, 1.25)] }));
add('instant-warning-does-not-truncate-active', area({ activeIntervals: [activation(1, 6), activation(3, 3), activation(9, 9, 1.25)] }));
add('same-start-instant-merge', area({ activeIntervals: [activation(4, 4, false), activation(4, 7, true)] }));
add('empty-activations', area({ activeIntervals: [] }));
add('non-default-flash-and-millisecond-rounding', area({ activeIntervals: [activation(4, 7, 0.8765), activation(9, 9, -1)] }),
    [0, 1, 1.1229, 1.123, 1.1960833333, 1.5615, 1.99999999, 2, 3, 5, 7]);
for (let easingType = 1; easingType <= 29; easingType++) {
    add(`standard-easing-${easingType}`, area({ moveXEvents: [event(-500, 400, 0, 4, { easingType, easingLeft: 0.12, easingRight: 0.87 })],
        rotateEvents: [event(-45, 80, 0, 4, { anchor: { x: 70, y: -90 }, easingType })] }));
}
for (const bezierPoints of [[0, 0, 1, 1], [0, -1, 0, 3], [1, -2, 1, 3], [0.2, 2, 0.9, -1]]) {
    add(`bezier-${bezierPoints.join(',')}`, area({ moveYEvents: [event(-200, 350, 0, 8, { bezier: 1, bezierPoints })] }));
}
const anchor = { x: 75, y: -25 };
add('independent-cumulative-anchors-with-gaps', area({
    moveXEvents: [event(0, -100, 0, 4), event(80, 160, 8, 12)],
    moveYEvents: [event(0, 50, 1, 3)],
    rotateEvents: [event(0, 90, 0, 4, { anchor }), event(120, -45, 7, 10, { anchor: { x: -90, y: 180 } })],
    scaleXEvents: [event(1, 2, 2, 4, { anchor }), event(3, -1, 6, 8, { anchor: { x: -90, y: 180 } })],
    scaleYEvents: [event(1, 3, 4, 8, { anchor: { x: -20, y: 80 } })],
}));
add('zero-and-negative-scales', area({ scaleXEvents: [event(0, -2, 0, 4, { anchor })], scaleYEvents: [event(-1, 2, 2, 6, { anchor })] }));
add('off-centre-initial-rotation-and-scale', area({ bottomLeft: { x: 200, y: -100 }, topRight: { x: 600, y: 20 },
    rotateEvents: [event(30, 60, 0, 4, { anchor })], scaleXEvents: [event(2, 3, 0, 4, { anchor })] }));
add('transform-step-chain-and-baseline-overrides', area({ rotateEvents: [
    event(0, 0, 0, 0, { anchor, transformStep: true, transformStart: 0, transformEnd: 90 }),
    event(90, 180, 0, 4, { anchor: { x: -20, y: 80 }, transformStart: 60, transformEnd: 200 }),
    event(200, 90, 5, 8, { anchor }),
], scaleXEvents: [event(1, 2, 0, 4, { anchor, transformStart: 0.5, transformEnd: 3 })] }));
const curves = [
    [{ x: 0, y: 0, mode: 'linear' }, { x: 1, y: 1, mode: 'linear' }],
    [{ x: 0, y: 0, mode: 'hold' }, { x: 0.4, y: 1.5, mode: 'linear' }, { x: 1, y: 1, mode: 'corner' }],
    [{ x: 0, y: 0, mode: 'auto' }, { x: 0.3, y: -0.5, mode: 'auto' }, { x: 0.8, y: 1.8, mode: 'auto' }, { x: 1, y: 1, mode: 'auto' }],
    [{ x: 0, y: 0, mode: 'corner', out: [0.1, 2] }, { x: 0.6, y: -0.5, mode: 'smooth', in: [0.3, -1], out: [0.9, 1] }, { x: 1, y: 1, in: [0.95, 2] }],
];
for (let i = 0; i < curves.length; i++) for (const cropped of [false, true]) {
    const customEasing = { curve: { version: 1, nodes: curves[i] }, ...(cropped ? { domain: [0.15, 0.8], sourceStart: -350, sourceEnd: 550 } : {}) };
    add(`animation-curve-${i}-${cropped}`, area({ moveXEvents: [event(-200, 300, 0, 4, { customEasing })] }));
}
const output = { source: { repository: 'kclg-YSJ/rpe-next', revision: execFileSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim() }, cases };
writeFileSync(process.argv[3], JSON.stringify(output) + '\n');
console.log(`${cases.length} source cases, ${cases.reduce((n, c) => n + c.samples.length, 0)} reference samples`);
