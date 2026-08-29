/**
 * presentationThemes.ts — 演示材料主题预设
 */

export interface PresentationTheme {
  name: string
  label: string
  cssVars: string
}

export const PRESENTATION_THEMES: PresentationTheme[] = [
  {
    name: 'light',
    label: '浅色',
    cssVars: `:root {
  --accent: #2563eb;
  --accent-light: #3b82f6;
  --bg: #ffffff;
  --bg-alt: #f9fafb;
  --text: #111827;
  --muted: #6b7280;
  --border: #e5e7eb;
  --card: #ffffff;
  --font: 'Inter', 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif;
  --slide-w: 1920px;
  --slide-h: 1080px;
}`
  },
  {
    name: 'dark',
    label: '深色',
    cssVars: `:root {
  --accent: #3b82f6;
  --accent-light: #60a5fa;
  --bg: #0f172a;
  --bg-alt: #1e293b;
  --text: #f1f5f9;
  --muted: #94a3b8;
  --border: #334155;
  --card: #1e293b;
  --font: 'Inter', 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif;
  --slide-w: 1920px;
  --slide-h: 1080px;
}`
  }
]

export function getThemeCss(themeName: string): string {
  const theme = PRESENTATION_THEMES.find((t) => t.name === themeName)
  return theme?.cssVars ?? PRESENTATION_THEMES[0].cssVars
}
