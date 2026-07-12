import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import AppIcon from '@/components/common/AppIcon.vue'

describe('AppIcon', () => {
  it('renders with default props', () => {
    const wrapper = mount(AppIcon, { props: { name: 'home' } })
    expect(wrapper.find('svg').exists()).toBe(true)
  })

  it('applies custom size', () => {
    const wrapper = mount(AppIcon, { props: { name: 'home', size: 24 } })
    const svg = wrapper.find('svg')
    expect(svg.attributes('width')).toBe('24')
    expect(svg.attributes('height')).toBe('24')
  })

  it('applies custom stroke width', () => {
    const wrapper = mount(AppIcon, { props: { name: 'home', stroke: 2 } })
    const svg = wrapper.find('svg')
    expect(svg.attributes('stroke-width')).toBe('2')
  })

  it('renders the correct icon path', () => {
    const wrapper = mount(AppIcon, { props: { name: 'check' } })
    const svg = wrapper.find('svg')
    expect(svg.html()).toContain('M4.5 12.75l6 6 9-13.5')
  })

  it('defaults to size 14', () => {
    const wrapper = mount(AppIcon, { props: { name: 'home' } })
    const svg = wrapper.find('svg')
    expect(svg.attributes('width')).toBe('14')
    expect(svg.attributes('height')).toBe('14')
  })
})