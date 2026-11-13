// @ts-nocheck
import { describe, expect, it } from 'vitest';

import {
  assertAria2Version,
  createPluginManifest,
  resolveReleaseTarget
} from '../scripts/package-plugin.mjs';

describe('Aria2 plugin package contract', () => {
  it('uses only theme and download domain permissions', () => {
    const target = resolveReleaseTarget('x86_64-unknown-linux-gnu');
    const files = {
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
});
