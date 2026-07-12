import { defineConfig } from 'vitepress'

export default defineConfig({
  base: '/artur/',
  title: 'Artur',
  description: 'Config-driven Rust HTTP gateway and package orchestrator',
  head: [['link', { rel: 'icon', href: '/artur/logo.svg' }]],
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/getting-started' },
      { text: 'Reference', link: '/reference/responses' },
      { text: 'GitHub', link: 'https://github.com/melonask/artur' }
    ],
    sidebar: {
      '/guide/': [
        { text: 'Guide', items: [
          { text: 'Getting started', link: '/guide/getting-started' },
          { text: 'Configuration', link: '/guide/configuration' },
          { text: 'Usage', link: '/guide/usage' }
        ] }
      ],
      '/reference/': [
        { text: 'Reference', items: [
          { text: 'Responses', link: '/reference/responses' },
          { text: 'Operations', link: '/reference/operations' }
        ] }
      ]
    },
    socialLinks: [{ icon: 'github', link: 'https://github.com/melonask/artur' }]
  }
})
