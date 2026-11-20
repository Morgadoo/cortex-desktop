// Generate the download-site manifest from final, signed installers.
// Usage: node scripts/make-version-manifest.mjs <macos|linux|windows> <file>...
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { basename } from 'node:path';

const formats = { macos: ['dmg'], linux: ['deb', 'rpm'], windows: ['exe'] };
const linuxLabels = { deb: '.deb (Ubuntu/Debian)', rpm: '.rpm (Fedora/RHEL)' };
const [platform, ...paths] = process.argv.slice(2);
const assert = (condition, message) => {
  if (!condition) throw new Error(message);
};

try {
  assert(formats[platform] && paths.length, 'usage: make-version-manifest.mjs <macos|linux|windows> <file>...');
  const { version } = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  const base = process.env.CORTEX_DOWNLOAD_BASE_URL || process.env.CORTEX_UPDATE_BASE_URL;
  assert(base, 'set CORTEX_DOWNLOAD_BASE_URL (or CORTEX_UPDATE_BASE_URL) to the public installer directory');
  const baseUrl = new URL(`${base.replace(/\/+$/, '')}/`);
  assert(['http:', 'https:'].includes(baseUrl.protocol) && !baseUrl.search && !baseUrl.hash, 'base URL must be an HTTP(S) directory without a query or fragment');

  // Metadata is shared across platforms or supplied separately to each job.
  const metadata = process.env.CORTEX_RELEASE_METADATA ? JSON.parse(await readFile(process.env.CORTEX_RELEASE_METADATA, 'utf8')) : {};
  assert(metadata && typeof metadata === 'object' && !Array.isArray(metadata), 'release metadata must be an object');
  const allowed = ['CategoryName', 'ReleaseDate', 'ReleaseNotes', 'SystemVersion', 'RolloutProportion', 'ManualUpdate', 'Checksum'];
  assert(
    Object.keys(metadata).every((key) => allowed.includes(key)),
    'unsupported release metadata field'
  );
  const release = {
    CategoryName: 'Stable',
    Version: version,
    ReleaseDate: new Date().toISOString().slice(0, 10),
    ...metadata,
  };
  assert(['Stable', 'EarlyAccess'].includes(release.CategoryName), 'invalid CategoryName');
  assert(
    typeof release.ReleaseDate === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(release.ReleaseDate) && new Date(`${release.ReleaseDate}T00:00:00Z`).toISOString().slice(0, 10) === release.ReleaseDate,
    'ReleaseDate must be a valid YYYY-MM-DD date'
  );
  if (release.RolloutProportion !== undefined) {
    assert(typeof release.RolloutProportion === 'number' && release.RolloutProportion >= 0 && release.RolloutProportion <= 1, 'RolloutProportion must be between 0 and 1');
  }
  if (release.ReleaseNotes !== undefined) {
    assert(
      Array.isArray(release.ReleaseNotes) &&
        release.ReleaseNotes.every(
          (note) => note && ['Improvement', 'Fix', 'Feature'].includes(note.Type) && Array.isArray(note.Notes) && note.Notes.every((text) => typeof text === 'string') && Object.keys(note).every((key) => ['Type', 'Notes'].includes(key))
        ),
      'invalid ReleaseNotes'
    );
  }
  if (release.SystemVersion !== undefined) {
    assert(
      release.SystemVersion &&
        typeof release.SystemVersion === 'object' &&
        !Array.isArray(release.SystemVersion) &&
        Object.entries(release.SystemVersion).every(([key, value]) => ['Minimum', 'Maximum'].includes(key) && typeof value === 'string' && /^\d+(?:\.\d+)*$/.test(value)),
      'invalid SystemVersion'
    );
  }
  if (release.ManualUpdate !== undefined) {
    assert(Array.isArray(release.ManualUpdate) && release.ManualUpdate.every((value) => typeof value === 'string'), 'ManualUpdate must be an array of strings');
  }
  if (release.Checksum !== undefined) assert(typeof release.Checksum === 'string', 'Checksum must be a string');

  const identifiers = new Set();
  release.File = [];
  for (const path of paths) {
    const name = basename(path);
    const identifier = name.split('.').at(-1);
    assert(formats[platform].includes(identifier), `unsupported ${platform} installer: ${name}`);
    assert(name.includes(version), `${name} does not carry version ${version}`);
    assert(!identifiers.has(identifier), `duplicate installer identifier: ${identifier}`);
    identifiers.add(identifier);
    const hash = createHash('sha512');
    for await (const chunk of createReadStream(path)) hash.update(chunk);
    release.File.push({ Url: new URL(encodeURIComponent(name), baseUrl).href, Sha512CheckSum: hash.digest('hex'), Identifier: platform === 'linux' ? linuxLabels[identifier] : identifier });
  }

  // Optional history is an existing platform version.json; replace a rerun of
  // the same version/channel and keep other releases in their existing order.
  const history = process.env.CORTEX_VERSION_HISTORY ? JSON.parse(await readFile(process.env.CORTEX_VERSION_HISTORY, 'utf8')) : { Releases: [] };
  assert(Array.isArray(history.Releases), 'history must contain a Releases array');
  const Releases = [release, ...history.Releases.filter((item) => item.Version !== version || item.CategoryName !== release.CategoryName)];
  await writeFile('version.json', `${JSON.stringify({ Releases }, null, 2)}\n`);
  console.log(`==> version.json (${platform}, ${version})`);
} catch (error) {
  console.error(`make-version-manifest: ${error.message}`);
  process.exitCode = 1;
}
