import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { describe, it } from 'node:test'

// The test script runs from apps/vscode (pnpm/turbo run it in the package directory).
const extensionDir = process.cwd()
const repoRoot = path.resolve(extensionDir, '..', '..')

const manifest = JSON.parse(readFileSync(path.join(extensionDir, 'package.json'), 'utf8'))

describe('Marketplace packaging (#31 #43 #46 #47 #48 #62)', () => {
  it('pins the extension to the workspace host (#47)', () => {
    assert.deepEqual(
      manifest.extensionKind,
      ['workspace'],
      'the extension shells out to slint against workspace files, so it must run where the files are',
    )
  })

  it('declares an existing PNG icon of at least 128x128 (#48)', () => {
    assert.equal(typeof manifest.icon, 'string', 'package.json must declare an "icon" field')
    const icon = readFileSync(path.join(extensionDir, manifest.icon))
    assert.equal(icon.readUInt32BE(0), 0x89504e47, 'the icon file must be a PNG')
    assert.equal(icon.readUInt32BE(4), 0x0d0a1a0a, 'the icon file must be a PNG')
    assert.equal(icon.readUInt32BE(12), 0x49484452, 'the PNG must start with an IHDR chunk')
    assert.ok(
      icon.readUInt32BE(16) >= 128 && icon.readUInt32BE(20) >= 128,
      'the Marketplace requires an icon of at least 128x128',
    )
  })

  it('declares the Marketplace polish fields (#62)', () => {
    assert.ok(Array.isArray(manifest.keywords) && manifest.keywords.length >= 3, 'keywords: []')
    for (const keyword of manifest.keywords) {
      assert.equal(typeof keyword, 'string')
      assert.ok(keyword.length > 0)
    }
    assert.equal(manifest.bugs?.url, 'https://github.com/MaximeGaudin/slint/issues')
    assert.match(manifest.galleryBanner?.color, /^#[0-9a-f]{6}$/i)
    assert.ok(['dark', 'light'].includes(manifest.galleryBanner?.theme))
  })

  it('ships a README.md for the Marketplace listing (#31)', () => {
    const readme = readFileSync(path.join(extensionDir, 'README.md'), 'utf8')
    assert.ok(readme.length > 500, 'the listing page renders from this file, it must not be a stub')
    assert.match(readme, /^## /m, 'the README must be structured with headings')
    assert.match(readme, /SKILL\.md/)
    assert.match(readme, /slint/)
  })

  it('ships a CHANGELOG.md with an entry for the current version (#46)', () => {
    const changelog = readFileSync(path.join(extensionDir, 'CHANGELOG.md'), 'utf8')
    assert.match(changelog, new RegExp(`^## \\[${manifest.version}\\]`, 'm'))
  })

  it('packages and validates the manifest on every CI run (#43)', () => {
    const ci = readFileSync(path.join(repoRoot, '.github', 'workflows', 'ci.yml'), 'utf8')
    assert.match(ci, /slint-vscode/)
    assert.match(
      ci,
      /package:vscode/,
      'CI must run the vsce packaging step to catch broken manifests',
    )
  })

  it('publishes the extension from the release workflow (#43)', () => {
    assert.match(
      manifest.scripts['publish:marketplace'],
      /vsce publish/,
      'publish:marketplace must publish with vsce',
    )
    assert.match(
      manifest.scripts['publish:openvsx'],
      /ovsx publish/,
      'publish:openvsx must publish with ovsx',
    )
    const release = readFileSync(path.join(repoRoot, '.github', 'workflows', 'release.yml'), 'utf8')
    assert.match(
      release,
      /publish:marketplace/,
      'the release workflow must publish to the VS Code Marketplace',
    )
    assert.match(release, /publish:openvsx/, 'the release workflow must publish to Open VSX')
    assert.match(release, /VSCE_PAT/)
    assert.match(release, /OVSX_PAT/)
  })
})
