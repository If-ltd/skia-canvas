const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');

const expectedApis = {
  auto: ['Vulkan', 'OpenGL'],
  d3d12: ['D3D12'],
  vulkan: ['Vulkan'],
  opengl: ['OpenGL'],
  cpu: [],
  invalid: [],
};

if (!process.argv[2]) {
  const actualApis = {};
  for (const mode of Object.keys(expectedApis)) {
    const env = { ...process.env };
    if (mode === 'auto') delete env.SKIA_CANVAS_BACKEND;
    else env.SKIA_CANVAS_BACKEND = mode;
    const output = execFileSync(process.execPath, [__filename, mode], { env, encoding: 'utf8' });
    const { api, renderer, error } = JSON.parse(output);
    actualApis[mode] = api;
    console.log(`${mode}: ${api || renderer}${error ? ` (${error})` : ''}`);
  }
  assert.equal(actualApis.auto, actualApis.vulkan ?? actualApis.opengl);
} else {
  verifyExport(process.argv[2]).catch(error => {
    console.error(error);
    process.exitCode = 1;
  });
}

async function verifyExport(mode) {
  const { Canvas, loadImage } = require('../lib');
  const canvas = new Canvas(32, 32);
  const context = canvas.getContext('2d');
  context.fillStyle = '#123456';
  context.fillRect(0, 0, 32, 32);

  const { api, renderer, error } = canvas.engine;
  if (renderer === 'GPU') {
    assert.ok(expectedApis[mode].includes(api), `Unexpected ${api} for ${mode}`);
  } else {
    assert.equal(renderer, 'CPU');
    assert.equal(api, null);
    if (mode !== 'cpu') assert.ok(error, 'CPU fallback should explain why the GPU was skipped');
  }
  assert.equal(canvas.gpu, renderer === 'GPU');
  const color = [0x12, 0x34, 0x56, 255];
  assert.deepEqual(Array.from(context.getImageData(8, 8, 1, 1).data), color);
  assert.deepEqual(Array.from(canvas.toBufferSync('raw').subarray(0, 4)), color);

  const png = await canvas.toBuffer('png');
  const decoded = new Canvas(32, 32, { gpu: false });
  const decodedContext = decoded.getContext('2d');
  decodedContext.drawImage(await loadImage(png), 0, 0);
  assert.equal(decoded.gpu, false);
  assert.equal(decoded.engine.api, null);
  assert.deepEqual(Array.from(decodedContext.getImageData(8, 8, 1, 1).data), color);
  console.log(JSON.stringify({ api, renderer, error }));
}
