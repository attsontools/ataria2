import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';

import { downloadVerified } from './runtime-download.mjs';

const [target, outputValue] = process.argv.slice(2);
if (!target || !outputValue) throw new Error('usage: build-aria2.mjs <target-triple> <output-path>');

const manifest = JSON.parse(readFileSync('runtime-manifest.json', 'utf8')).aria2;
const output = resolve(outputValue);
const temporary = mkdtempSync(resolve(tmpdir(), 'ataria2-runtime-'));
mkdirSync(dirname(output), { recursive: true });

try {
  if (target.includes('windows')) {
    const name = `aria2-${manifest.version}-win-64bit-build1.zip`;
    const origin = `https://github.com/aria2/aria2/releases/download/release-${manifest.version}/${name}`;
    const archive = await downloadVerified(origin, manifest.windowsAssetSha256);
    const archivePath = resolve(temporary, name);
    writeFileSync(archivePath, archive);
    run('7z', ['x', '-y', `-o${temporary}`, archivePath]);
    const source = resolve(temporary, name.replace(/\.zip$/, ''));
    copyFileSync(resolve(source, 'aria2c.exe'), output);
    copyFileSync(resolve(source, 'COPYING'), resolve(dirname(output), 'aria2-COPYING'));
    copyFileSync(resolve(source, 'LICENSE.OpenSSL'), resolve(dirname(output), 'aria2-LICENSE.OpenSSL'));
  } else {
    const name = `aria2-${manifest.version}.tar.gz`;
    const origin = `https://github.com/aria2/aria2/releases/download/release-${manifest.version}/${name}`;
    const archive = await downloadVerified(origin, manifest.sourceSha256);
    const archivePath = resolve(temporary, name);
    writeFileSync(archivePath, archive);
    run('tar', ['-xzf', archivePath, '-C', temporary]);
    const source = resolve(temporary, `aria2-${manifest.version}`);
    const prefix = process.env.ATARIA2_NATIVE_RUNTIME_PREFIX;
    if (!prefix) throw new Error('ATARIA2_NATIVE_RUNTIME_PREFIX is required to build aria2');
    const environment = {
      ...process.env,
      ARIA2_STATIC: 'yes',
      CPPFLAGS: `${process.env.CPPFLAGS || ''} -I${resolve(prefix, 'include')}`.trim(),
      LDFLAGS: `${process.env.LDFLAGS || ''} -L${resolve(prefix, 'lib')}`.trim(),
      PKG_CONFIG_PATH: [resolve(prefix, 'lib/pkgconfig'), resolve(prefix, 'share/pkgconfig'), process.env.PKG_CONFIG_PATH]
        .filter(Boolean).join(':')
    };
    const tls = target.includes('apple')
      ? ['--with-appletls', '--without-openssl']
      : ['--without-appletls', '--with-openssl', '--with-ca-bundle=/etc/ssl/certs/ca-certificates.crt'];
    run(resolve(source, 'configure'), [
      '--disable-nls', '--without-gnutls', '--without-libnettle', '--without-libgmp',
      '--without-libgcrypt', '--with-sqlite3', '--with-libxml2', '--without-libexpat',
      '--with-libcares', '--with-libz', '--with-libssh2', ...tls
    ], source, environment);
    run('make', ['-j2'], source, environment);
    copyFileSync(resolve(source, 'src/aria2c'), output);
    copyFileSync(resolve(source, 'COPYING'), resolve(dirname(output), 'aria2-COPYING'));
    chmodSync(output, 0o755);
  }
  const version = spawnSync(output, ['--version'], { encoding: 'utf8' });
  if (version.status !== 0 || !version.stdout.includes(`aria2 version ${manifest.version}`)) {
    throw new Error(`aria2 version smoke failed: ${version.stderr || version.stdout}`);
  }
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

function run(program, args, cwd = process.cwd(), env = process.env) {
  const result = spawnSync(program, args, { cwd, env, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program} failed with exit code ${result.status}`);
}
