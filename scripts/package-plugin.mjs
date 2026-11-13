import { spawnSync } from 'node:child_process';
import { createHash, createPrivateKey, createPublicKey } from 'node:crypto';
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { parseManifest } from '@attson/plugin-sdk';
import { signManifest, verifyManifestSignature } from '@attson/plugin-sdk/node';

const TARGETS = {
  'aarch64-apple-darwin': ['macos', 'aarch64', 'ataria2-service', 'aria2c'],
  'x86_64-apple-darwin': ['macos', 'x86_64', 'ataria2-service', 'aria2c'],
  'aarch64-unknown-linux-gnu': ['linux', 'aarch64', 'ataria2-service', 'aria2c'],
  'x86_64-unknown-linux-gnu': ['linux', 'x86_64', 'ataria2-service', 'aria2c'],
  'x86_64-pc-windows-msvc': ['windows', 'x86_64', 'ataria2-service.exe', 'aria2c.exe']
};

export function resolveReleaseTarget(targetTriple) {
  const value = TARGETS[targetTriple];
  if (!value) throw new Error(`Unsupported release target: ${targetTriple}`);
  return {
    targetTriple,
    targetOs: value[0],
    targetArch: value[1],
    serviceFileName: value[2],
    runtimeFileName: value[3]
  };
}

export function createPluginManifest({ version, target, publisherKeyId, files }) {
  const serviceEntry = `native/${target.serviceFileName}`;
  return parseManifest({
    schemaVersion: 1,
    appId: 'com.attson.aria2',
    name: 'Aria2 下载',
    version,
    publisher: { id: 'com.attson', name: 'Attson', keyId: publisherKeyId },
    target: { os: target.targetOs, arch: target.targetArch },
    hostApi: { min: '2.0.0', max: '2.x' },
    ui: { entry: 'ui/index.html' },
    permissions: ['host.theme.read', 'host.download.tasks'],
    nativeService: {
      entry: serviceEntry,
      version,
      sha256: files[serviceEntry],
      protocolVersion: 1
    },
    files
  });
}

export async function buildTestPackage({ root, version, targetTriple, runtimePath }) {
  const target = resolveReleaseTarget(targetTriple);
  const runtime = JSON.parse(await readFile(path.join(root, 'runtime-manifest.json'), 'utf8')).aria2;
  verifyRuntime(runtimePath, runtime.version);
  const stage = await mkdtemp(path.join(tmpdir(), 'ataria2-plugin-'));
  const artifacts = path.join(root, 'artifacts');
  const output = path.join(
    artifacts,
    `AT.Aria2_${version}_${target.targetOs}_${target.targetArch}.atpkg`
  );
  const { privateKey, publicKey } = localTestSigningKey();
  const keyId = 'attool-p0-test';
  try {
    run('cargo', ['build', '--release', '--bin', 'ataria2-service', '--target', targetTriple], root);
    await mkdir(path.join(stage, 'ui'), { recursive: true });
    await mkdir(path.join(stage, 'native'), { recursive: true });
    await mkdir(path.join(stage, 'licenses'), { recursive: true });
    await mkdir(artifacts, { recursive: true });
    await cp(path.join(root, 'dist-plugin'), path.join(stage, 'ui'), { recursive: true });
    await cp(path.join(root, 'LICENSE'), path.join(stage, 'LICENSE'));
    await cp(
      path.join(root, 'target', targetTriple, 'release', target.serviceFileName),
      path.join(stage, 'native', target.serviceFileName)
    );
    await cp(runtimePath, path.join(stage, 'native', target.runtimeFileName));
    await stageAria2License(root, stage);

    const files = {};
    for (const relative of await collect(stage)) {
      files[relative] = sha256(await readFile(path.join(stage, relative)));
    }
    const manifest = createPluginManifest({ version, target, publisherKeyId: keyId, files });
    const signature = signManifest(manifest, privateKey);
    if (!verifyManifestSignature(manifest, signature, publicKey)) {
      throw new Error('local test signature verification failed');
    }
    await writeFile(path.join(stage, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    await writeFile(path.join(stage, 'manifest.sig'), `${signature}\n`);
    await rm(output, { force: true });
    run('zip', ['-X', '-D', '-q', output, ...await collect(stage)], stage);
    return { output, manifest };
  } finally {
    await rm(stage, { recursive: true, force: true });
  }
}

export function assertAria2Version(output, expectedVersion) {
  if (!output.includes(`aria2 version ${expectedVersion}`)) {
    throw new Error(`Expected aria2 ${expectedVersion}`);
  }
}

function verifyRuntime(runtimePath, expectedVersion) {
  const result = spawnSync(runtimePath, ['--version'], { encoding: 'utf8' });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`aria2 version smoke failed with exit code ${result.status}`);
  }
  assertAria2Version(result.stdout, expectedVersion);
}

async function stageAria2License(root, stage) {
  const runtime = JSON.parse(await readFile(path.join(root, 'runtime-manifest.json'), 'utf8')).aria2;
  const name = `aria2-${runtime.version}.tar.gz`;
  const origin = `https://github.com/aria2/aria2/releases/download/release-${runtime.version}/${name}`;
  const archive = await downloadVerified(origin, runtime.sourceSha256);
  const temporary = await mkdtemp(path.join(tmpdir(), 'ataria2-license-'));
  try {
    const archivePath = path.join(temporary, name);
    await writeFile(archivePath, archive);
    run('tar', ['-xzf', archivePath, '-C', temporary, `aria2-${runtime.version}/COPYING`]);
    await cp(
      path.join(temporary, `aria2-${runtime.version}`, 'COPYING'),
      path.join(stage, 'licenses', 'aria2-COPYING')
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function downloadVerified(origin, expectedSha256) {
  let lastError;
  for (const url of [origin, `https://gh-proxy.com/${origin}`]) {
    try {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const bytes = Buffer.from(await response.arrayBuffer());
      const actual = sha256(bytes);
      if (actual !== expectedSha256) throw new Error(`checksum mismatch: ${actual}`);
      return bytes;
    } catch (error) {
      lastError = error;
    }
  }
  throw new Error(`unable to download verified aria2 archive: ${lastError}`);
}

async function collect(directory, prefix = '') {
  const output = [];
  for (const entry of await readdir(path.join(directory, prefix), { withFileTypes: true })) {
    const relative = path.posix.join(prefix, entry.name);
    if (entry.isDirectory()) output.push(...await collect(directory, relative));
    else output.push(relative);
  }
  return output.sort();
}

function sha256(content) {
  return createHash('sha256').update(content).digest('hex');
}

function localTestSigningKey() {
  const seed = Buffer.from('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60', 'hex');
  const publicBytes = Buffer.from('d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a', 'hex');
  const privateKey = createPrivateKey({
    key: {
      kty: 'OKP',
      crv: 'Ed25519',
      d: seed.toString('base64url'),
      x: publicBytes.toString('base64url')
    },
    format: 'jwk'
  });
  return { privateKey, publicKey: createPublicKey(privateKey) };
}

function rustHostTarget() {
  const result = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
  if (result.status !== 0) throw new Error('rustc -vV failed');
  const target = result.stdout.match(/^host: (.+)$/m)?.[1];
  if (!target) throw new Error('Unable to determine Rust host target');
  return target;
}

function run(program, args, cwd = process.cwd()) {
  const result = spawnSync(program, args, { cwd, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program} failed`);
}

async function main() {
  if (!process.argv.includes('--test-key')) throw new Error('only --test-key packaging is available in P1');
  const root = path.resolve(import.meta.dirname, '..');
  const packageJson = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
  const runtimePath = process.env.ATARIA2_RUNTIME_PATH;
  if (!runtimePath) throw new Error('ATARIA2_RUNTIME_PATH is required');
  const result = await buildTestPackage({
    root,
    version: packageJson.version,
    targetTriple: process.env.ATARIA2_RELEASE_TARGET || rustHostTarget(),
    runtimePath
  });
  console.log(result.output);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
