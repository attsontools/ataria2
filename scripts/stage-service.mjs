import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';

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

function rustHostTarget() {
  const output = execFileSync('rustc', ['-vV'], { encoding: 'utf8' });
  const match = output.match(/^host: (.+)$/m);
  if (!match) throw new Error('Unable to determine Rust host target');
  return match[1];
}
