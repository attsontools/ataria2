import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { downloadVerified } from './runtime-download.mjs';

const outputDirectory = process.argv[2];
if (!outputDirectory) throw new Error('usage: fetch-aria2-source.mjs <output-directory>');
const root = path.resolve(import.meta.dirname, '..');
const manifest = JSON.parse(await readFile(path.join(root, 'runtime-manifest.json'), 'utf8')).aria2;
const sourceName = `aria2-${manifest.version}.tar.gz`;
const origin = `https://github.com/aria2/aria2/releases/download/release-${manifest.version}/${sourceName}`;
const bytes = await downloadVerified(origin, manifest.sourceSha256);
await mkdir(outputDirectory, { recursive: true });
await writeFile(path.join(outputDirectory, `aria2-${manifest.version}-source.tar.gz`), bytes);
