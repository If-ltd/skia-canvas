import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

const platform = 'win32';
const arch = 'x64';
const upstreamVersion = JSON.parse(readFileSync('package.json', 'utf8')).version;
const tag = process.env.GITHUB_REF_NAME;
const sourceCommit = process.env.GITHUB_SHA;
if (process.platform !== platform || process.arch !== arch
    || !tag?.startsWith(`v${upstreamVersion}-moda.`)
    || !/^\d+$/.test(tag.slice(`v${upstreamVersion}-moda.`.length))
    || !/^[0-9a-f]{40}$/.test(sourceCommit ?? '')) {
    throw new Error('Expected a tagged Moda Windows x64 release build');
}

const sha256 = data => createHash('sha256').update(data).digest('hex');
const binary = readFileSync('lib/skia.node');
const archive = gzipSync(binary, { level: 9 });
const basename = `skia-canvas-${platform}-${arch}`;
const asset = `${basename}.node.gz`;
writeFileSync(asset, archive);
writeFileSync(`${basename}.json`, JSON.stringify({
    tag, upstreamVersion, sourceCommit, platform, arch, asset,
    sha256: sha256(binary), bytes: binary.length,
    archiveSha256: sha256(archive), archiveBytes: archive.length,
}, null, 2) + '\n');
