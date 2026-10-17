import { box, line, led, triR, F, F2, A, AD, S, W } from './iconArtBase'

/* ============ 服务器与存储 ============ */

const server: Record<string, string> = {
  /* 服务器塔：三层机位 + 指示灯 */
  server: `${box(8, 3, 16, 26, 2, F2)}${line('M8 12 H24', S, 1.5)}${line('M8 21 H24', S, 1.5)}${led(12, 7.5, A)}${led(12, 16.5, A)}${led(12, 25.5, A)}${line('M16 7 H20', S, 1.5)}${line('M16 16 H20', S, 1.5)}${line('M16 25 H20', S, 1.5)}`,

  /* Web 服务器：塔 + 地球徽标 */
  web_server: `${box(7, 4, 15, 19, 2, F2)}${line('M7 11 H22', S, 1.5)}${line('M7 17 H22', S, 1.5)}${led(11, 7.5, A)}${led(11, 14, A)}${`<circle cx="23" cy="24" r="6.5" fill="${F}" stroke="${A}" stroke-width="2"/>`}${line('M16.8 24 H29.2', A, 1.3)}${`<ellipse cx="23" cy="24" rx="3" ry="6.5" fill="none" stroke="${S}" stroke-width="1.3"/>`}`,

  /* 数据库：圆柱 + 层线 */
  database: `${`<path d="M6 7.5 V24 A10 3.6 0 0 0 26 24 V7.5" fill="${F}" stroke="${S}" stroke-width="2"/>`}${`<ellipse cx="16" cy="7.5" rx="10" ry="3.6" fill="${F2}" stroke="${S}" stroke-width="2"/>`}${line('M6 13.6 A10 3.6 0 0 0 26 13.6', S, 1.4)}${line('M6 19 A10 3.6 0 0 0 26 19', A, 1.8)}`,

  /* SQL 数据库：圆柱 + 强调点行 */
  sql: `${`<path d="M6 7.5 V24 A10 3.6 0 0 0 26 24 V7.5" fill="${F}" stroke="${S}" stroke-width="2"/>`}${`<ellipse cx="16" cy="7.5" rx="10" ry="3.6" fill="${F2}" stroke="${S}" stroke-width="2"/>`}${led(11, 14, A)}${led(16, 15, A)}${led(21, 14, A)}${line('M6 20 A10 3.6 0 0 0 26 20', S, 1.4)}`,

  /* 邮件服务器：信封 */
  mail: `${box(3.5, 7.5, 25, 17, 2, F2)}${line('M4.5 9 L16 17.5 L27.5 9', S, 1.8)}${line('M4.5 23.5 L12.5 17', S, 1.5)}${line('M27.5 23.5 L19.5 17', S, 1.5)}${led(25, 23, A)}`,

  /* 大型机：宽机柜 + 格栅 */
  mainframe: `${box(3, 5, 26, 22, 2, F2)}<rect x="6" y="7.5" width="20" height="3" rx="1.5" fill="${A}"/>${[7.5, 11, 14.5, 18, 21.5].map((x) => line(`M${x} 14 V23`, S, 1.5)).join('')}${led(24.5, 9, W, 1)}`,

  /* 集群：三节点 + 中心枢纽 */
  cluster: `${box(3, 4, 10, 7.5, 1.5, F2)}${box(19, 4, 10, 7.5, 1.5, F2)}${box(11, 21, 10, 7.5, 1.5, F2)}${line('M13 9.5 L14.5 13')}${line('M19 9.5 L17.5 13')}${line('M16 21 V19')}${led(16, 15.5, A, 4)}${led(7, 7.7, A, 1.5)}${led(25, 7.7, A, 1.5)}${led(16, 24.7, A, 1.5)}`,

  /* 服务器农场：四格阵列 */
  farm: `${box(4, 4, 11, 10, 1.5, F2)}${box(17, 4, 11, 10, 1.5, F2)}${box(4, 18, 11, 10, 1.5, F2)}${box(17, 18, 11, 10, 1.5, F2)}${led(7, 7, A, 1.5)}${led(7, 21, A, 1.5)}${led(20, 7, AD, 1.5)}${led(20, 21, AD, 1.5)}${line('M11 13 H17', S, 1.5)}${line('M15 14 V18', S, 1.5)}`,

  /* 存储阵列：三层盘位 */
  storage_array: `${box(4, 5, 24, 6, 2, F2)}${box(4, 13, 24, 6, 2, F2)}${box(4, 21, 24, 6, 2, F2)}${led(8, 8, A, 1.5)}${led(8, 16, A, 1.5)}${led(8, 24, A, 1.5)}${line('M12 8 H24', S, 1.5)}${line('M12 16 H24', S, 1.5)}${line('M12 24 H24', S, 1.5)}`,

  /* 机柜：柜体 + 层板 */
  rack: `${box(5, 2, 22, 28, 2)}${line('M5 9 H27', S, 1.5)}${line('M5 16 H27', S, 1.5)}${line('M5 23 H27', S, 1.5)}${led(8, 5.5, A, 1.5)}${led(8, 12.5, A, 1.5)}${led(8, 19.5, A, 1.5)}${led(8, 26.5, A, 1.5)}${line('M14 5.5 H24', S, 1.3)}${line('M14 12.5 H24', S, 1.3)}${line('M14 19.5 H24', S, 1.3)}`,

  /* 空机柜：柜体 + 导轨 */
  rack_empty: `${box(5, 2, 22, 28, 2)}${line('M10 4 V28', S, 1.5)}${line('M22 4 V28', S, 1.5)}${[7, 15, 23].map((y) => line(`M10 ${y} H12`, S, 1.5)).join('')}${[7, 15, 23].map((y) => line(`M20 ${y} H22`, S, 1.5)).join('')}`,

  /* UPS：机箱 + 电池 + 闪电 */
  ups: `${box(4, 9, 24, 14, 2, F2)}${`<rect x="7.5" y="12.5" width="13" height="7" rx="1.5" fill="${F}" stroke="${S}" stroke-width="1.5"/>`}${`<rect x="9" y="14" width="7.5" height="4" rx="0.8" fill="${AD}"/>`}${line('M24 12.5 l-3.2 4.4 h3 l-3.2 4.4', A, 1.8)}${led(7.5, 21, AD, 0)}`,

  /* PDU：插座条 */
  pdu: `${box(2.5, 12, 27, 8, 4, F2)}${`<circle cx="8" cy="16" r="2.3" fill="${F}" stroke="${S}" stroke-width="1.5"/>`}${`<circle cx="16" cy="16" r="2.3" fill="${F}" stroke="${S}" stroke-width="1.5"/>`}${`<circle cx="24" cy="16" r="2.3" fill="${F}" stroke="${S}" stroke-width="1.5"/>`}`,

  /* 虚拟机：双窗口叠放 */
  vm: `${box(4, 4, 16, 13, 2, F2)}${box(12, 15, 16, 13, 2, F, S, 2)}${led(7.5, 7.5, A, 1.5)}${line('M7 11.5 H17', S, 1.5)}${led(15.5, 18.5, A, 1.5)}${line('M15 22.5 H25', S, 1.5)}`,

  /* 硬盘：盘体 + 盘片 */
  harddrive: `${box(3.5, 8, 25, 16, 3, F2)}${`<circle cx="14" cy="16" r="5.4" fill="${F}" stroke="${S}" stroke-width="1.5"/>`}${led(14, 16, AD, 1.8)}${line('M14 16 L23 10.5', S, 1.5)}${led(25, 11, A)}`,

  /* NAS：盘体 + 网口 */
  nas: `${box(3, 9, 19, 14, 2, F2)}${`<circle cx="11" cy="16" r="4.6" fill="${F}" stroke="${S}" stroke-width="1.5"/>`}${led(11, 16, AD, 1.6)}${box(24, 12.5, 6, 7, 1.2, A, A, 1)}${led(7, 12, A, 1.4)}`
}

/* ============ 终端与办公 ============ */

const terminal: Record<string, string> = {
  /* PC：显示器 + 底座 */
  pc: `${box(4, 4, 24, 16, 2, F2)}${`<rect x="6.5" y="6.5" width="19" height="11" rx="1" fill="${F}" stroke="${S}" stroke-width="1.4"/>`}${line('M16 20 V25', S, 2)}${line('M11 25.5 H21', S, 2.4)}${`<rect x="8.5" y="14" width="7" height="1.6" rx="0.8" fill="${A}"/>`}`,

  /* 工作站：显示器 + 机箱 */
  workstation: `${box(3, 5, 17, 13, 2, F2)}${`<rect x="5" y="7" width="13" height="9" rx="1" fill="${F}" stroke="${S}" stroke-width="1.4"/>`}${line('M11.5 18 V22', S, 2)}${line('M8 22.5 H15', S, 2)}${box(22, 5, 7, 22, 1.5, F2)}${led(25.5, 8.5, A, 1.5)}${line('M23.5 13 H27.5', S, 1.3)}${line('M23.5 16 H27.5', S, 1.3)}`,

  /* 笔记本 */
  laptop: `${box(6, 5, 20, 14, 1.5, F2)}${`<rect x="8" y="7" width="16" height="10" rx="1" fill="${F}" stroke="${S}" stroke-width="1.3"/>`}${line('M8 14 H14', A, 1.6)}${`<path d="M3.5 25 L6.5 19 H25.5 L28.5 25 Z" fill="${F2}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>`}${led(16, 22.5, A, 1.4)}`,

  /* 平板 */
  tablet: `${box(7.5, 3, 17, 26, 3, F2)}${`<rect x="9.5" y="6" width="13" height="17" rx="1" fill="${F}" stroke="${S}" stroke-width="1.3"/>`}${led(16, 26, AD, 1.6)}${led(13, 8.5, A, 1.2)}`,

  /* 手机 */
  phone: `${box(9.5, 3, 13, 26, 3, F2)}${line('M13 6 H19', S, 1.5)}${`<rect x="11.5" y="8.5" width="9" height="14" rx="1" fill="${F}" stroke="${S}" stroke-width="1.2"/>`}${led(16, 26, AD, 1.5)}`,

  /* 打印机 */
  printer: `${box(9, 4, 14, 9, 1, F2)}${line('M11.5 7 H18.5', S, 1.4)}${box(4, 13, 24, 11, 2, F)}${led(8, 17, A)}${line('M13 17 H24', S, 1.4)}${box(8, 22, 16, 7, 1, F2)}${line('M11 25 H21', S, 1.4)}`,

  /* IP 摄像机 */
  ip_camera: `${line('M16 4 V8.5', S, 2)}${line('M11 4 H21', S, 2.4)}${box(6, 10, 20, 12, 6, F2)}${`<circle cx="13.5" cy="16" r="4.4" fill="${F}" stroke="${S}" stroke-width="1.6"/>`}${led(13.5, 16, AD, 2.2)}${line('M21 16 H26', A, 2)}${led(27.5, 16, A, 1.4)}`,

  /* 显示器 */
  monitor: `${box(3.5, 5, 25, 17, 2, F2)}${`<rect x="6" y="7.5" width="20" height="12" rx="1" fill="${F}" stroke="${S}" stroke-width="1.4"/>`}${box(12, 24, 8, 3, 1.5, F2)}${`<rect x="8" y="16" width="8" height="1.8" rx="0.9" fill="${A}"/>`}`,

  /* USB 集线器 */
  usb_hub: `${box(3, 11.5, 26, 9, 4.5, F2)}${box(7, 14, 4.5, 4, 1, A, A, 1)}${box(14, 14, 4.5, 4, 1, A, A, 1)}${box(21, 14, 4.5, 4, 1, AD, AD, 1)}${line('M16 20.5 V26.5', S, 2)}${line('M12.5 27 H19.5', S, 2)}`
}

/* ============ 建筑与场地 ============ */

const site: Record<string, string> = {
  /* 大型建筑 */
  building_lg: `${box(7, 3, 18, 26, 1.5, F2)}${[6, 11.5, 17].map((y) => [10, 18].map((x) => `<rect x="${x}" y="${y}" width="5" height="3.5" rx="0.8" fill="${F}" stroke="${S}" stroke-width="1.3"/>`).join('')).join('')}<rect x="18" y="17" width="5" height="3.5" rx="0.8" fill="${A}"/>${box(13, 24, 6, 5, 0.8, F, S, 1.3)}`,

  /* 商业楼宇：双塔 */
  building_biz: `${box(3.5, 11, 11, 17, 1.5, F2)}${box(16, 5, 12.5, 23, 1.5, F2)}${[14, 18.5, 23].map((y) => `<rect x="6" y="${y}" width="5.5" height="3" rx="0.7" fill="${F}" stroke="${S}" stroke-width="1.2"/>`).join('')}${[8.5, 13.5, 18.5, 23.5].map((y) => `<rect x="18.5" y="${y}" width="6" height="3" rx="0.7" fill="${F}" stroke="${S}" stroke-width="1.2"/>`).join('')}<rect x="18.5" y="18.5" width="6" height="3" rx="0.7" fill="${A}"/>`,

  /* 学校 */
  school: `${line('M16 3 V11', S, 2)}${`<path d="M16 3.5 l6 2.2 -6 2.2 Z" fill="${A}"/>`}${box(5, 12, 22, 16, 1.5, F2)}${`<path d="M3 12 L16 6.5 L29 12 Z" fill="${F}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>`}${[15, 19.5].map((y) => [9, 14.5, 20].map((x) => `<rect x="${x}" y="${y}" width="3.5" height="3.5" rx="0.7" fill="${F}" stroke="${S}" stroke-width="1.2"/>`).join('')).join('')}${box(14, 24, 4.5, 4, 0.7, A, A, 1)}`,

  /* 安保建筑 */
  secure_building: `${box(5, 5, 17, 21, 1.5, F2)}${[8.5, 13, 17.5].map((y) => `<rect x="8" y="${y}" width="4.5" height="3.2" rx="0.7" fill="${F}" stroke="${S}" stroke-width="1.2"/>`).join('')}${`<path d="M21 17 l7 2.6 v4.4 c0 4.2 -3 6.3 -7 8 c-4 -1.7 -7 -3.8 -7 -8 v-4.4 z" fill="${A}" stroke="${F}" stroke-width="1.6" stroke-linejoin="round"/>`}${line('M18.2 23.6 l1.8 1.8 l3.6 -4', W, 1.8)}`,

  /* 家庭 */
  home: `${`<path d="M4.5 15.5 L16 6 L27.5 15.5 V27 H4.5 Z" fill="${F}" stroke="${S}" stroke-width="2" stroke-linejoin="round"/>`}${box(8, 17, 5.5, 5.5, 0.8, A, A, 1)}${box(13.5, 21, 5, 6, 0.8, F2, S, 1.5)}${line('M21 20 H24 V24 H21 Z', S, 1.5)}`,

  /* 信息插座 */
  data_jack: `${box(6.5, 6.5, 19, 19, 2.5, F2)}${box(10.5, 11, 11, 8, 1.2, F, S, 1.5)}${`<path d="M13 19 v3 h6 v-3" fill="${F}" stroke="${S}" stroke-width="1.5"/>`}${`<rect x="13" y="13" width="6" height="4" rx="0.6" fill="${A}"/>`}${led(16, 23, AD, 1.4)}`
}

export const ICONS_B: Record<string, string> = { ...server, ...terminal, ...site }
