// @ts-nocheck
import { generateKeyPairSync } from 'node:crypto';
import { describe, expect, it } from 'vitest';

import {
  assertAria2Version,
  createPluginManifest,
  loadPublisherSigningConfig,
  requiredPayloadPaths,
  resolveReleaseTarget
} from '../scripts/package-plugin.mjs';

describe('Aria2 plugin package contract', () => {
  it('uses only theme and download domain permissions', () => {
    const target = resolveReleaseTarget('x86_64-unknown-linux-gnu');
    const files = {
      LICENSE: 'e'.repeat(64),
      'ui/index.html': 'a'.repeat(64),
      'native/ataria2-service': 'b'.repeat(64),
      'native/aria2c': 'c'.repeat(64),
      'licenses/aria2-COPYING': 'd'.repeat(64)
    };

    const manifest = createPluginManifest({
      version: '0.1.0-alpha.0',
      target,
      publisherKeyId: 'attool-p1-local-test',
      files
    });

    expect(manifest.permissions).toEqual(['host.theme.read', 'host.download.tasks']);
    expect(manifest.nativeService.entry).toBe('native/ataria2-service');
    expect(manifest.nativeService.sha256).toBe('b'.repeat(64));
    expect(manifest.files['native/aria2c']).toBe('c'.repeat(64));
  });

  it('maps target-specific service and runtime names', () => {
    expect(resolveReleaseTarget('x86_64-pc-windows-msvc')).toMatchObject({
      targetOs: 'windows',
      targetArch: 'x86_64',
      serviceFileName: 'ataria2-service.exe',
      runtimeFileName: 'aria2c.exe'
    });
    expect(() => resolveReleaseTarget('wasm32-unknown-unknown')).toThrow(/Unsupported/);
  });

  it('rejects a runtime that is not the pinned aria2 version', () => {
    expect(() => assertAria2Version('aria2 version 1.37.0\n', '1.37.0')).not.toThrow();
    expect(() => assertAria2Version('aria2 version 1.36.0\n', '1.37.0'))
      .toThrow(/Expected aria2 1.37.0/);
  });

  it('requires matching production Publisher key material', () => {
    const { privateKey, publicKey } = generateKeyPairSync('ed25519');
    const privatePem = privateKey.export({ type: 'pkcs8', format: 'pem' }).toString();
    const publicJwk = publicKey.export({ format: 'jwk' });
    const publicRaw = Buffer.from(publicJwk.x, 'base64url').toString('base64');
    const signing = loadPublisherSigningConfig({
      ATARIA2_PUBLISHER_KEY_ID: 'attson-aria2-2026-01',
      ATARIA2_PUBLISHER_PRIVATE_KEY: privatePem,
      ATARIA2_PUBLISHER_PUBLIC_KEY: publicRaw
    });

    expect(signing.keyId).toBe('attson-aria2-2026-01');
    expect(() => loadPublisherSigningConfig({})).toThrow(/KEY_ID is required/);
    expect(() => loadPublisherSigningConfig({
      ATARIA2_PUBLISHER_KEY_ID: 'attson-aria2-2026-01',
      ATARIA2_PUBLISHER_PRIVATE_KEY: privatePem,
      ATARIA2_PUBLISHER_PUBLIC_KEY: Buffer.alloc(32).toString('base64')
    })).toThrow(/does not match/);
  });

  it('requires target-specific service, runtime, and license payloads', () => {
    expect(requiredPayloadPaths(resolveReleaseTarget('x86_64-unknown-linux-gnu'))).toEqual([
      'LICENSE',
      'licenses/aria2-COPYING',
      'native/aria2c',
      'native/ataria2-service',
      'ui/index.html'
    ]);
    expect(requiredPayloadPaths(resolveReleaseTarget('x86_64-pc-windows-msvc')))
      .toContain('licenses/aria2-LICENSE.OpenSSL');
  });
});
