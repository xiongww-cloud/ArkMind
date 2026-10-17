import { initIcons, iconUri, iconSize } from './graph/iconArt'
import { buildCategories } from './graph/icons'

async function main() {
  const app = document.getElementById('app')!
  await initIcons()
  const grid = document.createElement('div')
  grid.className = 'grid'
  for (const cat of buildCategories()) {
    for (const item of cat.items) {
      if (!item.src) continue
      for (const dark of [false, true]) {
        const cell = document.createElement('div')
        cell.className = 'cell' + (dark ? ' dark' : '')
        const img = document.createElement('img')
        img.src = item.src
        const meta = document.createElement('div')
        meta.className = 'meta'
        const big = document.createElement('img')
        big.src = item.src
        big.className = 'lg'
        const span = document.createElement('span')
        span.textContent = `${item.label} ${item.w}×${item.h}`
        meta.appendChild(span)
        cell.appendChild(img)
        cell.appendChild(big)
        grid.appendChild(cell)
      }
    }
  }
  app.appendChild(grid)
}

main()
