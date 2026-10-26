import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { nextTick } from 'vue'
import DeviceTable from '@/components/project/DeviceTable.vue'

function dev(n: number) {
  return {
    uid: 'd' + n,
    rowType: 'device',
    model: 'M' + n,
    customName: '',
    quantity: 1,
    unit: '台',
    unitPrice: 1,
    mergedText: '',
    params: []
  }
}

function fire(el: Element, type: string, init: MouseEventInit = {}) {
  const ev = new MouseEvent(type, { bubbles: true, cancelable: true, ...init })
  el.dispatchEvent(ev)
  return ev
}

function selected(rows: any[]) {
  return rows.map((r: any) => {
    const s: string = r.attributes('style') || ''
    return s.indexOf('rgba(191') >= 0
  })
}

describe('DeviceTable shift range select', () => {
  it('plain click then shift+click selects the range', async () => {
    const items = [dev(1), dev(2), dev(3), dev(4)]
    const wrapper = mount(DeviceTable, {
      props: { items },
      global: { plugins: [createPinia()] }
    })
    await nextTick()
    const rows = () => wrapper.findAll('[data-item-idx]')
    expect(rows().length).toBe(4)

    fire(rows()[0].element, 'mousedown')
    fire(rows()[0].element, 'click')
    await nextTick()
    console.log('after click1:', JSON.stringify(selected(rows())))

    // 修饰键 mousedown 必须 preventDefault——否则浏览器会拉起原生文本选择（Shift 多选时整行被蓝选）
    const sm = fire(rows()[2].element, 'mousedown', { shiftKey: true })
    expect(sm.defaultPrevented).toBe(true)
    fire(rows()[2].element, 'click', { shiftKey: true })
    await nextTick()
    const sel = selected(rows())
    console.log('after shift click3:', JSON.stringify(sel))

    expect(sel[0]).toBe(true)
    expect(sel[1]).toBe(true)
    expect(sel[2]).toBe(true)
    expect(sel[3]).toBe(false)

    // Ctrl+click 加选 d4、减选 d1（Excel 语义）
    fire(rows()[3].element, 'mousedown', { ctrlKey: true })
    fire(rows()[3].element, 'click', { ctrlKey: true })
    await nextTick()
    expect(selected(rows())).toEqual([true, true, true, true])
    fire(rows()[0].element, 'mousedown', { ctrlKey: true })
    fire(rows()[0].element, 'click', { ctrlKey: true })
    await nextTick()
    expect(selected(rows())).toEqual([false, true, true, true])
    wrapper.unmount()
  })
})
