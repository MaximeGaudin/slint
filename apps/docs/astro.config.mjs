import { defineConfig } from 'astro/config'
import mdx from '@astrojs/mdx'

/**
 * Static docs site. Prose lives in MDX; the rule catalogue is still rendered from
 * `slint rules --json` so the site and the binary cannot drift apart.
 */
export default defineConfig({
  site: 'https://slint.dev',
  integrations: [mdx()],
  build: {
    format: 'directory',
  },
})
