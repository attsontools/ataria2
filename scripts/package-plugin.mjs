import { spawnSync } from 'node:child_process';
import { createHash, createPrivateKey, createPublicKey } from 'node:crypto';
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { parseManifest } from '@attson/plugin-sdk';
import { signManifest, verifyManifestSignature } from '@attson/plugin-sdk/node';

import { downloadVerified } from './runtime-download.mjs';

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

export function requiredPayloadPaths(target) {
  return [
    'LICENSE',
    'licenses/aria2-COPYING',
    'licenses/aria2-LICENSE.OpenSSL',
    `native/${target.runtimeFileName}`,
    `native/${target.serviceFileName}`,
    'ui/index.html'
  ].sort();
}

export function createPluginManifest({ version, target, publisherKeyId, files }) {
  for (const relative of requiredPayloadPaths(target)) {
    if (!files[relative]) throw new Error(`Plugin payload is missing ${relative}`);
  }
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

export function assertPluginManifestContract(manifest, expected) {
  const serviceEntry = `native/${expected.target.serviceFileName}`;
  requireEqual(manifest.appId, 'com.attson.aria2', 'app ID');
  requireEqual(manifest.version, expected.version, 'version');
  requireEqual(manifest.publisher.id, 'com.attson', 'Publisher ID');
  requireEqual(manifest.publisher.keyId, expected.publisherKeyId, 'Publisher Key ID');
  requireEqual(manifest.target.os, expected.target.targetOs, 'target OS');
  requireEqual(manifest.target.arch, expected.target.targetArch, 'target architecture');
  requireEqual(manifest.hostApi.min, '2.0.0', 'minimum Host API');
  requireEqual(manifest.hostApi.max, '2.x', 'maximum Host API');
  requireEqual(
    JSON.stringify(manifest.permissions),
    JSON.stringify(['host.theme.read', 'host.download.tasks']),
    'permissions'
  );
  requireEqual(manifest.nativeService?.entry, serviceEntry, 'native service entry');
  requireEqual(manifest.nativeService?.version, expected.version, 'native service version');
  requireEqual(manifest.nativeService?.protocolVersion, 1, 'native service protocol version');
  requireEqual(manifest.nativeService?.sha256, manifest.files[serviceEntry], 'native service hash');
  for (const relative of requiredPayloadPaths(expected.target)) {
    if (!manifest.files[relative]) throw new Error(`Plugin payload is missing ${relative}`);
  }
}

export function loadPublisherSigningConfig(env) {
  const keyId = env.ATARIA2_PUBLISHER_KEY_ID?.trim();
  if (!keyId) throw new Error('ATARIA2_PUBLISHER_KEY_ID is required');
  const encodedKey = env.ATARIA2_PUBLISHER_PRIVATE_KEY?.trim();
  if (!encodedKey) throw new Error('ATARIA2_PUBLISHER_PRIVATE_KEY is required');
  const encodedPublicKey = env.ATARIA2_PUBLISHER_PUBLIC_KEY?.trim();
  if (!encodedPublicKey) throw new Error('ATARIA2_PUBLISHER_PUBLIC_KEY is required');
  if (!/^[A-Za-z0-9+/]{43}=$/.test(encodedPublicKey)) {
    throw new Error('ATARIA2_PUBLISHER_PUBLIC_KEY must be a raw Ed25519 public key');
  }
  let privateKey;
  try {
    privateKey = createPrivateKey(encodedKey);
  } catch {
    throw new Error('ATARIA2_PUBLISHER_PRIVATE_KEY must be an Ed25519 private key');
  }
  if (privateKey.asymmetricKeyType !== 'ed25519') {
    throw new Error('ATARIA2_PUBLISHER_PRIVATE_KEY must be an Ed25519 private key');
  }
  const publicKey = createPublicKey(privateKey);
  const derived = publicKey.export({ format: 'jwk' });
  const derivedRaw = Buffer.from(derived.x, 'base64url').toString('base64');
  if (derivedRaw !== encodedPublicKey) {
    throw new Error('Publisher private key does not match configured public key');
  }
  return { keyId, privateKey, publicKey, testKey: false };
}

export async function buildTestPackage({ root, version, targetTriple, runtimePath }) {
  return buildPluginPackage({
    root,
    version,
    targetTriple,
    runtimePath,
    signing: localTestSigningKey()
  });
}

export async function buildPluginPackage({ root, version, targetTriple, runtimePath, signing }) {
  const target = resolveReleaseTarget(targetTriple);
  const runtime = JSON.parse(await readFile(path.join(root, 'runtime-manifest.json'), 'utf8')).aria2;
  verifyRuntime(runtimePath, runtime.version);
  const stage = await mkdtemp(path.join(tmpdir(), 'ataria2-plugin-'));
  const artifacts = path.join(root, 'artifacts');
  const output = path.join(
    artifacts,
    `AT.Aria2_${version}_${target.targetOs}_${target.targetArch}.atpkg`
  );
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
    await stageAria2Licenses(root, stage, target, runtimePath, signing.testKey);

    const files = {};
    for (const relative of await collect(stage)) {
      files[relative] = sha256(await readFile(path.join(stage, relative)));
    }
    const manifest = createPluginManifest({
      version,
      target,
      publisherKeyId: signing.keyId,
      files
    });
    const signature = signManifest(manifest, signing.privateKey);
    if (!verifyManifestSignature(manifest, signature, signing.publicKey)) {
      throw new Error('local test signature verification failed');
    }
    await writeFile(path.join(stage, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    await writeFile(path.join(stage, 'manifest.sig'), `${signature}\n`);
    await rm(output, { force: true });
    await archive(stage, output);
    await verifyPluginPackage(output, signing.publicKey, {
      version,
      publisherKeyId: signing.keyId,
      target
    });
    return { output, manifest };
  } catch (error) {
    await rm(output, { force: true });
    throw error;
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

async function stageAria2Licenses(root, stage, target, runtimePath, testKey) {
  if (!testKey) {
    const runtimeDirectory = path.dirname(runtimePath);
    await cp(
      path.join(runtimeDirectory, 'aria2-COPYING'),
      path.join(stage, 'licenses', 'aria2-COPYING')
    );
    await cp(
      path.join(runtimeDirectory, 'aria2-LICENSE.OpenSSL'),
      path.join(stage, 'licenses', 'aria2-LICENSE.OpenSSL')
    );
    return;
  }
  const runtime = JSON.parse(await readFile(path.join(root, 'runtime-manifest.json'), 'utf8')).aria2;
  const name = `aria2-${runtime.version}.tar.gz`;
  const origin = `https://github.com/aria2/aria2/releases/download/release-${runtime.version}/${name}`;
  const archive = await downloadVerified(origin, runtime.sourceSha256);
  const temporary = await mkdtemp(path.join(tmpdir(), 'ataria2-license-'));
  try {
    const archivePath = path.join(temporary, name);
    await writeFile(archivePath, archive);
    run('tar', [
      '-xzf', archivePath, '-C', temporary,
      `aria2-${runtime.version}/COPYING`,
      `aria2-${runtime.version}/LICENSE.OpenSSL`
    ]);
    await cp(
      path.join(temporary, `aria2-${runtime.version}`, 'COPYING'),
      path.join(stage, 'licenses', 'aria2-COPYING')
    );
    await cp(
      path.join(temporary, `aria2-${runtime.version}`, 'LICENSE.OpenSSL'),
      path.join(stage, 'licenses', 'aria2-LICENSE.OpenSSL')
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

export async function verifyPluginPackage(packagePath, publicKey, expected) {
  const expectedName = `AT.Aria2_${expected.version}_${expected.target.targetOs}_${expected.target.targetArch}.atpkg`;
  if (path.basename(packagePath) !== expectedName) {
    throw new Error(`Package file name must be ${expectedName}`);
  }
  const extracted = await mkdtemp(path.join(tmpdir(), 'ataria2-verify-'));
  try {
    extract(packagePath, extracted);
    const manifest = parseManifest(
      JSON.parse(await readFile(path.join(extracted, 'manifest.json'), 'utf8'))
    );
    const signature = (await readFile(path.join(extracted, 'manifest.sig'), 'utf8')).trim();
    if (!verifyManifestSignature(manifest, signature, publicKey)) {
      throw new Error('Plugin manifest signature is invalid');
    }
    assertPluginManifestContract(manifest, expected);
    const actualFiles = (await collect(extracted))
      .filter((relative) => relative !== 'manifest.json' && relative !== 'manifest.sig');
    const declaredFiles = Object.keys(manifest.files).sort();
    if (JSON.stringify(actualFiles) !== JSON.stringify(declaredFiles)) {
      throw new Error('Package files do not match the manifest');
    }
    for (const relative of declaredFiles) {
      const actualHash = sha256(await readFile(path.join(extracted, relative)));
      if (actualHash !== manifest.files[relative]) {
        throw new Error(`Package hash mismatch for ${relative}`);
      }
    }
    return manifest;
  } finally {
    await rm(extracted, { recursive: true, force: true });
  }
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
  return {
    keyId: 'attool-p0-test',
    privateKey,
    publicKey: createPublicKey(privateKey),
    testKey: true
  };
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

async function archive(stage, output) {
  const entries = await collect(stage);
  if (process.platform === 'win32') {
    run('7z', ['a', '-tzip', '-mx=9', output, ...entries], stage);
  } else {
    run('zip', ['-X', '-D', '-q', output, ...entries], stage);
  }
}

function extract(packagePath, destination) {
  if (process.platform === 'win32') {
    run('7z', ['x', '-y', `-o${destination}`, packagePath]);
  } else {
    run('unzip', ['-q', packagePath, '-d', destination]);
  }
}

function requireEqual(actual, expected, label) {
  if (actual !== expected) {
    throw new Error(`Package ${label} mismatch: expected ${expected}, received ${actual}`);
  }
}

async function main() {
  const root = path.resolve(import.meta.dirname, '..');
  const packageJson = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
  const runtimePath = process.env.ATARIA2_RUNTIME_PATH;
  if (!runtimePath) throw new Error('ATARIA2_RUNTIME_PATH is required');
  const testKey = process.argv.includes('--test-key');
  const result = await buildPluginPackage({
    root,
    version: packageJson.version,
    targetTriple: process.env.ATARIA2_RELEASE_TARGET || rustHostTarget(),
    runtimePath,
    signing: testKey ? localTestSigningKey() : loadPublisherSigningConfig(process.env)
  });
  console.log(result.output);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
