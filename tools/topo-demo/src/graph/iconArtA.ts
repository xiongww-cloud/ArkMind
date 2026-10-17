import { box, line, hArrow, vArrow, portRow, led, wifi, switchChassis, triL, triR, F, F2, A, AD, S, W } from './iconArtBase'

/* ============ 路由与交换 ============ */

const routing: Record<string, string> = {
  /* 路由器：机箱 + 交叉双向箭头 */
  router: `${box(3, 8, 26, 16, 4)}${hArrow(7, 25, 13.5)}${vArrow(16, 11.5, 20.5, S)}`,

  /* 交换机：机箱 + 转发箭头 + 端口排 */
  switch: switchChassis(),

  /* 三层交换机：机箱 + 端口排 + 右侧垂直箭头 */
  switch_l3: `${box(3, 9, 26, 14)}${hArrow(7, 19.5, 13.5)}${vArrow(25, 12, 20)}${portRow(17, 3, 4.5)}`,

  /* 框式交换机：机箱 + 4 槽位 + 底部强调条 */
  modular_switch: `${box(3, 7, 26, 18, 3, F2)}${[6.5, 12, 17.5, 23]
    .map((x) => `<rect x="${x}" y="10.5" width="4" height="9.5" rx="1" fill="${F}" stroke="${S}" stroke-width="1.5"/>`)
    .join('')}${led(7, 21.5, A)}${led(10.5, 21.5, A)}${led(25, 21.5, AD)}`,

  /* 工业交换机：加厚机箱 + 散热鳍 */
  industrial_switch: `${box(3, 9, 26, 14, 2)}${[7, 10, 13].map((y) => line(`M7 ${y} H16`, S, 1.5)).join('')}${led(24, 13, A)}${led(24, 19, A)}${portRow(17, 3, 19)}`,

  /* FC 交换机：机箱 + SFP 光口笼（方笼圆芯） + 状态灯 */
  fc_switch: `${box(3, 8, 26, 16, 2)}${[5.5, 11, 16.5, 22]
    .map(
      (x) =>
        `<rect x="${x}" y="11" width="4.5" height="6" rx="1" fill="${F}" stroke="${S}" stroke-width="1.4"/>` +
        `<circle cx="${x + 2.25}" cy="14" r="1.2" fill="${A}"/>`
    )
    .join('')}${led(7, 21, A)}${led(10.5, 21, A)}${led(14, 21, AD)}${line('M20 21 H26', S, 1.4)}`,

  /* 网桥：两台设备 + 链接 */
  bridge: `${box(3, 10, 11, 12, 2)}${box(18, 10, 11, 12, 2)}${led(7, 14, A)}${led(22, 14, A)}${line('M14 16 H18', A)}${led(16, 16, AD, 1.8)}`,

  /* 中继器：机箱 + 双向信号弧 */
  repeater: `${box(3, 9, 26, 14)}${line('M12.5 12 A6 6 0 0 0 12.5 20', S, 2)}${line('M19.5 12 A6 6 0 0 1 19.5 20', A)}${led(16, 16, AD, 2)}`,

  /* 集中器：三线汇聚 + 强调出口 */
  concentrator: `${box(3, 9, 26, 14)}${line('M6 12.5 L17 16')}${line('M6 16 H17')}${line('M6 19.5 L17 16')}${line('M18 16 H24', A)}${triR(27, 16, 3.2, A)}`,

  /* 调制解调器：机箱 + 调制波 */
  modem: `${box(3, 9, 26, 14)}${line('M7 16 q3.2 -5.5 6.4 0 t6.4 0 t5.2 0', A)}${led(6.5, 12, AD)}${led(25.5, 12, AD)}`,

  /* 光纤收发器：双腔 + 光/电信号 */
  media_converter: `${box(3, 11, 11, 10, 2)}${box(18, 11, 11, 10, 2)}${line('M13.5 14 H18.5', A)}${triR(18.5, 14, 3, A)}${line('M13.5 18 q2.5 3 5 0', S, 1.5)}`,

  /* 无线 AP：底座 + 双弧 */
  ap: `${box(6, 21, 20, 7, 3.5, F2)}${led(10, 24.5, AD)}${wifi(16, 18)}`,

  /* 室外 AP：半球 + 立杆 + 弧 */
  ap_outdoor: `${line('M16 16 V27')}${box(11, 27, 10, 3, 1.5, F2)}${`<path d="M7 16 A9 9 0 0 1 25 16 Z" fill="${F}" stroke="${S}" stroke-width="2"/>`}${led(16, 13, AD)}${line('M11 10 A7 7 0 0 1 21 10', A)}`,

  /* 无线路由器：机箱 + 天线弧 + 端口 */
  wireless_router: `${box(3, 13, 26, 11, 3)}${wifi(16, 9.5)}${portRow(19.5, 5, 5)}`,

  /* 无线路由器（保留语义别名） */
  wireless_ap: `${box(6, 21, 20, 7, 3.5, F2)}${led(10, 24.5, AD)}${wifi(16, 18)}`
}

/* ============ 安全 ============ */

const security: Record<string, string> = {
  /* 防火墙：砖墙 + 一块强调砖 */
  firewall: `${box(3, 8, 26, 16, 2, F2)}${line('M3 13.5 H29', S, 1.5)}${line('M3 18.5 H29', S, 1.5)}${line('M11 8 V13.5', S, 1.5)}${line('M21 13.5 V18.5', S, 1.5)}${line('M11 18.5 V24', S, 1.5)}<rect x="13" y="13.5" width="8" height="5" fill="${A}"/>`,

  /* 防火墙（防护型）：盾牌 + 砖线 */
  firewall2: `<path d="M16 3.5 L27 7.5 V15.5 C27 22.5 22 26.8 16 28.5 C10 26.8 5 22.5 5 15.5 V7.5 Z" fill="${F}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>${line('M9 13 H23', S, 1.5)}${line('M9 18 H23', S, 1.5)}${line('M16 9 V13', S, 1.5)}${line('M11 18 V23', S, 1.5)}${line('M21 18 V23', A, 2)}`,

  /* VPN 网关：机箱 + 锁 */
  vpn_router: `${box(3, 9, 26, 14)}${`<rect x="12.5" y="15" width="7" height="5.5" rx="1" fill="${A}"/>`}${line('M13.75 15 v-1.8 a2.25 2.25 0 0 1 4.5 0 v1.8', S, 1.8)}${led(6.5, 12, AD)}${led(25.5, 12, AD)}`,

  /* UTM：机箱 + 规则线 + 小盾 */
  utm: `${box(3, 9, 26, 14)}${line('M6.5 13.5 H17', S, 1.5)}${line('M6.5 16.5 H15', S, 1.5)}${line('M6.5 19.5 H17', S, 1.5)}<path d="M22 11.5 l4.5 1.7 v3.4 c0 3.1 -2 4.7 -4.5 5.9 c-2.5 -1.2 -4.5 -2.8 -4.5 -5.9 v-3.4 z" fill="${A}"/>`,

  /* 入侵检测：机箱 + 眼睛 */
  ids: `${box(3, 9, 26, 14)}${`<path d="M9 16 q7 -5.5 14 0 q-7 5.5 -14 0 Z" fill="${F2}" stroke="${S}" stroke-width="1.5"/>`}${led(16, 16, AD, 2.6)}`,

  /* 代理网关：机箱 + 分流箭头 */
  proxy: `${box(3, 9, 26, 14)}<rect x="15.2" y="11.5" width="1.6" height="9" rx="0.8" fill="${S}"/>${line('M6.5 13.5 H13.5', A)}${triR(13.5, 13.5, 3, A)}${line('M18.5 18.5 H25.5', S)}${triR(25.5, 18.5, 3, S)}`,

  /* 证书：文档 + 印章 */
  cert: `${box(6, 3.5, 20, 25, 2, F2)}${line('M9.5 9.5 H22.5', S, 1.5)}${line('M9.5 13.5 H19', S, 1.5)}${led(22, 22, A, 5)}${line('M19.7 22 l1.7 1.7 l3 -3.4', W, 2)}`,

  /* 密钥：环 + 柄 */
  key: `${`<circle cx="11" cy="21" r="6.2" fill="${A}"/>`}${`<circle cx="11" cy="21" r="2.2" fill="${F2}"/>`}${line('M15.4 16.6 L27 5', S, 2.4)}${line('M21.5 10.5 L24.5 13.5', S, 2.4)}${line('M24.7 7.3 L27.4 10', S, 2.4)}`,

  /* 隧道：拱门 + 穿行箭头 */
  tunnel: `<path d="M5 27 V16.5 A11 11 0 0 1 27 16.5 V27" fill="${F}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>${line('M10 27 V17.5 A6 6 0 0 1 22 17.5 V27', A, 2)}${line('M11.5 22.5 H20.5', S, 1.8)}${triR(20.5, 22.5, 2.6, S)}`
}

/* ============ 云与互联网 ============ */

const CLOUD_D = 'M10 24 H21.5 a5.2 5.2 0 0 0 0.6 -10.35 A7.4 7.4 0 0 0 9.4 12.9 A5.6 5.6 0 0 0 10 24 Z'

const cloud: Record<string, string> = {
  /* 云 */
  cloud: `<path d="${CLOUD_D}" fill="${F}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>${led(16, 19, A)}`,

  /* 云服务器：云 + 服务条 */
  cloud_server: `<path d="M9.5 14.5 H20.8 a4.6 4.6 0 0 0 0.5 -9.2 A6.6 6.6 0 0 0 9 9.4 A5 5 0 0 0 9.5 14.5 Z" fill="${F}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>${box(7, 19, 18, 8, 2, F2)}${led(10.5, 23, A)}${line('M14 21.5 H22', S, 1.5)}${line('M14 24.5 H20', S, 1.5)}`,

  /* 云存储：云 + 磁盘条 */
  cloud_storage: `<path d="M9.5 14.5 H20.8 a4.6 4.6 0 0 0 0.5 -9.2 A6.6 6.6 0 0 0 9 9.4 A5 5 0 0 0 9.5 14.5 Z" fill="${F}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>${box(7, 19, 18, 8, 2, F2)}${led(10.5, 23, A)}${line('M14 23 H22', A, 1.5)}`,

  /* 互联网：地球 */
  internet: `${`<circle cx="16" cy="16" r="12" fill="${F}" stroke="${S}" stroke-width="2"/>`}${line('M4 16 H28', A, 1.8)}${`<ellipse cx="16" cy="16" rx="5" ry="12" fill="none" stroke="${S}" stroke-width="1.5"/>`}${line('M7 10 H25', S, 1.2)}${line('M7 22 H25', S, 1.2)}`,

  /* 全球网络：地球 + 轨道强调 */
  internet_globe: `${`<circle cx="16" cy="16" r="9" fill="${F}" stroke="${S}" stroke-width="2"/>`}${line('M7 16 H25', A, 1.5)}${`<ellipse cx="16" cy="16" rx="4" ry="9" fill="none" stroke="${S}" stroke-width="1.5"/>`}${`<ellipse cx="16" cy="16" rx="14.5" ry="6" fill="none" stroke="${A}" stroke-width="1.5" transform="rotate(-24 16 16)"/>`}${led(27.5, 11.5, AD, 1.8)}`
}

export const ICONS_A: Record<string, string> = { ...routing, ...security, ...cloud }
