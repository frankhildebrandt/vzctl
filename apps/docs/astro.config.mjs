import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://frankhildebrandt.github.io',
  base: '/vzctl/',
  integrations: [
    starlight({
      title: 'vzctl',
      description:
        'macOS DevStack-Supervisor: Git-native Multi-VM Environments, Hypernetwork, DNS und Docker.',
      defaultLocale: 'root',
      locales: {
        root: {
          label: 'Deutsch',
          lang: 'de',
        },
      },
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/frankhildebrandt/vzctl',
        },
      ],
      customCss: [
        './src/styles/tokens.css',
        './src/styles/starlight.css',
      ],
      components: {
        Header: './src/components/StarlightHeader.astro',
        Footer: './src/components/StarlightFooter.astro',
        ThemeSelect: './src/components/Empty.astro',
      },
      sidebar: [
        {
          label: 'Start',
          items: [{ label: 'Übersicht', slug: 'docs' }],
        },
        {
          label: 'Guides',
          items: [
            { label: 'Installation', slug: 'docs/guides/install' },
            { label: 'Quickstart', slug: 'docs/guides/quickstart' },
            { label: 'Hypernetwork', slug: 'docs/guides/hypernetwork' },
            { label: 'Netze & DNS', slug: 'docs/guides/networks-dns' },
            { label: 'Docker', slug: 'docs/guides/docker' },
            { label: 'Images', slug: 'docs/guides/images' },
            { label: 'CLI', slug: 'docs/guides/cli' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Architektur', slug: 'docs/reference/architecture' },
            { label: 'Beispiel edge-dmz', slug: 'docs/reference/example-edge-dmz' },
          ],
        },
      ],
      expressiveCode: {
        themes: ['github-light'],
      },
    }),
  ],
});
