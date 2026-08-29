import mdx from '@astrojs/mdx'
import sitemap from '@astrojs/sitemap'
import { defineConfig } from 'astro/config'

/**
 * Static docs site. Prose lives in MDX; the rule catalogue is still rendered from
 * `slint rules --json` so the site and the binary cannot drift apart.
 */
export default defineConfig({
  site: 'https://slint.dev',
  integrations: [mdx(), sitemap()],
  build: {
    format: 'directory',
  },
})
