export type ThemeName = 'white' | 'paper' | 'dark'

export interface ThemeTokens {
  bg: string
  labelFg: string
  labelCandidates: string[]
  regionFill: string
  regionStroke: string
  regionLabel: string
  regionLabelCandidates: string[]
  gridFine: [string, string]
  gridDot: string
  edgeDefault: string
  edgeLabelFg: string
  pillBg: string
  pillStroke: string
}

export const THEMES: Record<ThemeName, ThemeTokens> = {
  white: {
    bg: '#ffffff',
    labelFg: '#26282c',
    labelCandidates: ['#26282c', '#e5e7eb', '#d1d5db'],
    regionFill: '#f4f6fa',
    regionStroke: '#c9d2df',
    regionLabel: '#8a94a6',
    regionLabelCandidates: ['#8a94a6', '#9aa4b5'],
    gridFine: ['#eef0f4', '#e1e5ec'],
    gridDot: '#d7dce4',
    edgeDefault: '#4b5563',
    edgeLabelFg: '#374151',
    pillBg: '#ffffff',
    pillStroke: '#d9dfe8'
  },
  paper: {
    bg: '#faf9f5',
    labelFg: '#2c2a24',
    labelCandidates: ['#26282c', '#2c2a24', '#e5e7eb'],
    regionFill: '#f1efe6',
    regionStroke: '#d4cfbe',
    regionLabel: '#96907c',
    regionLabelCandidates: ['#8a94a6', '#96907c'],
    gridFine: ['#eceae1', '#e3e0d2'],
    gridDot: '#d9d5c6',
    edgeDefault: '#4b5563',
    edgeLabelFg: '#3d3a30',
    pillBg: '#ffffff',
    pillStroke: '#ddd8c8'
  },
  dark: {
    bg: '#1f2733',
    labelFg: '#e5e7eb',
    labelCandidates: ['#26282c', '#2c2a24', '#e5e7eb'],
    regionFill: '#273244',
    regionStroke: '#46536a',
    regionLabel: '#9aa4b5',
    regionLabelCandidates: ['#8a94a6', '#96907c', '#9aa4b5'],
    gridFine: ['#2b3543', '#323e4f'],
    gridDot: '#3a4658',
    edgeDefault: '#cbd5e1',
    edgeLabelFg: '#d1d5db',
    pillBg: '#3b465c',
    pillStroke: '#4d5b73'
  }
}

let current: ThemeName = 'white'

export function initThemeFromStorage() {
  try {
    const v = localStorage.getItem('topo.theme')
    if (v === 'white' || v === 'paper' || v === 'dark') current = v
  } catch {
    /* ignore */
  }
  return current
}

export function getTheme(): ThemeName {
  return current
}

export function setTheme(name: ThemeName) {
  current = name
  try {
    localStorage.setItem('topo.theme', name)
  } catch {
    /* ignore */
  }
}

export function themeTokens(): ThemeTokens {
  return THEMES[current]
}
