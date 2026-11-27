import { execFileSync } from 'node:child_process';
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync
} from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir, tmpdir } from 'node:os';

import { downloadVerified } from './runtime-download.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const release = process.argv.includes('--release');
const target = process.env.TAURI_ENV_TARGET_TRIPLE || rustHostTarget();
const extension = target.includes('windows') ? '.exe' : '';
const profile = release ? 'release' : 'debug';
const cargoArgs = ['build', '--bin', 'ataria2-service', '--target', target];
if (release) cargoArgs.push('--release');
execFileSync('cargo', cargoArgs, { cwd: root, stdio: 'inherit' });

const source = resolve(root, 'target', target, profile, `ataria2-service${extension}`);
const destination = resolve(
  root,
  'src-tauri',
  'binaries',
  `ataria2-service-${target}${extension}`
);
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);
if (!extension) chmodSync(destination, 0o755);

const runtimeSource = process.env.ATARIA2_RUNTIME_PATH || resolve(
  homedir(), '.cache', 'attool', 'runtime', 'aria2', '1.37.0', 'bin', `aria2c${extension}`
);
const runtimeVersion = execFileSync(runtimeSource, ['--version'], { encoding: 'utf8' });
if (!runtimeVersion.includes('aria2 version 1.37.0')) {
  throw new Error(`Expected aria2 1.37.0 at ${runtimeSource}`);
}
const runtimeDestination = resolve(
  root,
  'src-tauri',
  'binaries',
  `aria2c-${target}${extension}`
);
copyFileSync(runtimeSource, runtimeDestination);
if (!extension) chmodSync(runtimeDestination, 0o755);
await stageLicenses(runtimeSource);

async function stageLicenses(runtimePath) {
  const licenses = resolve(root, 'src-tauri', 'binaries', 'licenses');
  mkdirSync(licenses, { recursive: true });
  const runtimeDirectory = dirname(runtimePath);
  const copying = resolve(runtimeDirectory, 'aria2-COPYING');
  const openSslNotice = resolve(runtimeDirectory, 'aria2-LICENSE.OpenSSL');
  if (existsSync(copying) && existsSync(openSslNotice)) {
    copyFileSync(copying, resolve(licenses, 'aria2-COPYING'));
    copyFileSync(openSslNotice, resolve(licenses, 'aria2-LICENSE.OpenSSL'));
  } else {
    const manifest = JSON.parse(readFileSync(resolve(root, 'runtime-manifest.json'), 'utf8')).aria2;
    const name = `aria2-${manifest.version}.tar.gz`;
    const origin = `https://github.com/aria2/aria2/releases/download/release-${manifest.version}/${name}`;
    const temporary = mkdtempSync(resolve(tmpdir(), 'ataria2-license-'));
    try {
      const archive = resolve(temporary, name);
      writeFileSync(archive, await downloadVerified(origin, manifest.sourceSha256));
      execFileSync('tar', [
        '-xzf', archive, '-C', temporary,
        `aria2-${manifest.version}/COPYING`,
        `aria2-${manifest.version}/LICENSE.OpenSSL`
      ]);
      copyFileSync(
        resolve(temporary, `aria2-${manifest.version}`, 'COPYING'),
        resolve(licenses, 'aria2-COPYING')
      );
      copyFileSync(
        resolve(temporary, `aria2-${manifest.version}`, 'LICENSE.OpenSSL'),
        resolve(licenses, 'aria2-LICENSE.OpenSSL')
      );
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }
}

function rustHostTarget() {
  const output = execFileSync('rustc', ['-vV'], { encoding: 'utf8' });
  const match = output.match(/^host: (.+)$/m);
  if (!match) throw new Error('Unable to determine Rust host target');
  return match[1];
}
