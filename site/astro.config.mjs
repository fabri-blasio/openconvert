// @ts-check
import { defineConfig, passthroughImageService } from 'astro/config';
import vercel from '@astrojs/vercel';
import { fileURLToPath } from 'node:url';

const REPO = fileURLToPath(new URL('..', import.meta.url));

// openconvert.dev — static, no server, no runtime, no third-party requests.
// See 10-WEBSITE §9: content pages ship 0 KB of JavaScript.
export default defineConfig({
  site: 'https://openconvert.dev',
  // STILL STATIC. `output: 'static'` with an adapter prerenders every page as
  // before; the adapter exists for exactly one route, `/api/contact`, which
  // opts out with `export const prerender = false`. The contact form needs a
  // server because the Resend key is a secret, and a secret in browser
  // JavaScript is a secret anyone can send mail with.
  //
  // Everything else — all fourteen pages — is still a file on disk, and the
  // gates still read them out of dist/.
  output: 'static',
  adapter: vercel({
    webAnalytics: { enabled: true },
  }),
  trailingSlash: 'never',
  build: {
    inlineStylesheets: 'always',
    format: 'file',
  },
  devToolbar: { enabled: false },
  compressHTML: true,

  image: {
    // NO SHARP. Astro's default image service pulls sharp, which bundles
    // libvips, which carried four high-severity CVEs at the version npm
    // resolved (GHSA-f88m-g3jw-g9cj). This site contains no images at all, so
    // the whole dependency was surface with no function.
    //
    // Worth stating plainly on a site whose argument is about parser CVEs in
    // image libraries: we were shipping a build that depended on one.
    service: passthroughImageService(),
  },

  vite: {
    server: {
      // release/privacy.json and models.toml live in the repository (release/
      // and root respectively) and are imported by /privacy and /models,
      // because both pages have to render the same bytes CI gates against
      // rather than a copy. The dev server
      // refuses reads outside its root unless the root is named.
      fs: { allow: [REPO] },
    },
  },
});
