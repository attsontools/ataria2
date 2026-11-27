// @ts-nocheck
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import {
  assertVersionAlignment,
  expectedReleaseAssets
} from '../scripts/verify-release.mjs';

describe('Aria2 release contract', () => {
  it('requires the tag and all four project versions to match', () => {
    expect(assertVersionAlignment({
      tagName: 'v0.1.0-alpha.1',
      packageVersion: '0.1.0-alpha.1',
      tauriVersion: '0.1.0-alpha.1',
      serviceCargoVersion: '0.1.0-alpha.1',
      tauriCargoVersion: '0.1.0-alpha.1'
    })).toBe('0.1.0-alpha.1');

    expect(() => assertVersionAlignment({
      tagName: 'v0.1.0-alpha.1',
      packageVersion: '0.1.0-alpha.0',
      tauriVersion: '0.1.0-alpha.1',
      serviceCargoVersion: '0.1.0-alpha.1',
      tauriCargoVersion: '0.1.0-alpha.1'
    })).toThrow(/must all match/);
  });

  it('requires five installers, five plugins, and the pinned source archive', () => {
    const assets = expectedReleaseAssets('0.1.0-alpha.1', '1.37.0');

    expect(assets).toHaveLength(11);
    expect(assets).toContain('AT.Aria2_0.1.0-alpha.1_windows_x86_64.exe');
    expect(assets).toContain('AT.Aria2_0.1.0-alpha.1_linux_aarch64.atpkg');
    expect(assets).toContain('aria2-1.37.0-source.tar.gz');
  });

  it('declares the five-target production build and tag-only release', () => {
    const workflow = readFileSync('.github/workflows/release.yml', 'utf8');

    for (const target of [
      'aarch64-apple-darwin',
      'x86_64-apple-darwin',
      'aarch64-unknown-linux-gnu',
      'x86_64-unknown-linux-gnu',
      'x86_64-pc-windows-msvc'
    ]) expect(workflow).toContain(target);
    expect(workflow).toContain('environment: prod');
    expect(workflow).toContain('ATARIA2_PUBLISHER_PRIVATE_KEY');
    expect(workflow).toContain('Build pinned aria2 runtime');
    expect(workflow).toContain('ATARIA2_REAL_ARIA2=$RUNTIME');
    expect(workflow).toContain("if: startsWith(github.ref, 'refs/tags/v')");
    expect(workflow).toContain('scripts/verify-release.mjs assets');
  });

  it('stages aria2 licenses into the standalone application bundle', () => {
    const stageService = readFileSync('scripts/stage-service.mjs', 'utf8');
    const tauriConfig = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));

    expect(stageService).toContain('aria2-COPYING');
    expect(stageService).toContain('aria2-LICENSE.OpenSSL');
    expect(tauriConfig.bundle.resources).toContain('binaries/licenses');
  });

  it('pins the plugin SDK to an immutable GitHub commit for clean release runners', () => {
    const packageJson = JSON.parse(readFileSync('package.json', 'utf8'));
    const lockfile = readFileSync('pnpm-lock.yaml', 'utf8');
    const dependency = packageJson.dependencies['@attson/plugin-sdk'];

    expect(dependency).toMatch(
      /^github:attsontools\/attool-plugin-sdk#[0-9a-f]{40}$/
    );
    expect(lockfile).toContain(
      `codeload.github.com/attsontools/attool-plugin-sdk/tar.gz/${dependency.split('#')[1]}`
    );
  });
});
