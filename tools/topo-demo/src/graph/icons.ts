import { iconUri, iconSize } from './iconArt'
import { scaleForModern, scaleForClassicPath, isEndDevice, isEndDeviceClassicPath, endFit } from './iconCatalog'
import { classicAsset } from './iconArt'

export interface IconItem {
  label: string
  src: string
  w: number
  h: number
}

/* 描述符：buildCategories 时才解析 src/尺寸（tabler 异步注册后才就绪） */
interface Desc {
  kind: 'd' | 't' | 'c'
  id: string
  label: string
}
const D = (id: string, label: string): Desc => ({ kind: 'd', id, label })
const T = (id: string, label: string): Desc => ({ kind: 't', id, label })
const C = (path: string, label: string): Desc => ({ kind: 'c', id: path, label })

export interface IconCategory {
  name: string
  title: string
  items: IconItem[]
}

const CATEGORIES_DATA: { name: string; title: string; items: Desc[] }[] = [
  {
    name: 'routing',
    title: '路由与交换',
    items: [
      D('router', '路由器'),
      D('switch', '交换机'),
      D('switch_l3', '三层交换机'),
      D('modular_switch', '框式交换机'),
      D('industrial_switch', '工业交换机'),
      D('fc_switch', 'FC交换机'),
      D('wireless_router', '无线路由器'),
      D('ap', '室内AP'),
      D('ap_outdoor', '室外AP'),
      D('modem', '调制解调器'),
      D('bridge', '网桥'),
      D('repeater', '中继器'),
      D('concentrator', '集中器'),
      D('media_converter', '光纤收发器')
    ]
  },
  {
    name: 'security',
    title: '安全',
    items: [
      D('firewall', '防火墙'),
      D('firewall2', '防火墙(防护)'),
      D('vpn_router', 'VPN网关'),
      D('utm', 'UTM'),
      D('ids', '入侵检测'),
      D('proxy', '代理网关'),
      D('cert', '证书'),
      D('key', '密钥'),
      D('tunnel', '隧道'),
      T('shield-check', '安全检查')
    ]
  },
  {
    name: 'cloud',
    title: '云与互联网',
    items: [
      D('cloud', '云'),
      D('cloud_server', '云服务器'),
      D('cloud_storage', '云存储'),
      D('internet', '互联网'),
      D('internet_globe', '全球网络')
    ]
  },
  {
    name: 'server',
    title: '服务器与存储',
    items: [
      D('server', '服务器'),
      D('web_server', 'Web服务器'),
      D('database', '数据库'),
      D('sql', 'SQL数据库'),
      D('mail', '邮件服务器'),
      D('mainframe', '大型机'),
      D('cluster', '集群'),
      D('farm', '服务器农场'),
      D('storage_array', '存储'),
      D('rack', '机柜'),
      D('rack_empty', '空机柜'),
      D('ups', 'UPS'),
      D('pdu', 'PDU'),
      D('vm', '虚拟机'),
      D('harddrive', '硬盘'),
      D('nas', 'NAS')
    ]
  },
  {
    name: 'terminal',
    title: '终端与办公',
    items: [
      D('pc', 'PC'),
      D('workstation', '工作站'),
      D('laptop', '笔记本'),
      D('tablet', '平板'),
      D('phone', '手机'),
      D('printer', '打印机'),
      D('ip_camera', 'IP摄像头'),
      D('monitor', '显示器'),
      D('usb_hub', 'USB集线器'),
      T('device-desktop', '桌面设备')
    ]
  },
  {
    name: 'site',
    title: '建筑与场地',
    items: [
      D('building_lg', '大型建筑'),
      D('building_biz', '商业楼宇'),
      D('school', '学校'),
      D('secure_building', '安保建筑'),
      D('home', '家庭'),
      D('data_jack', '信息插座'),
      T('map-pin', '位置标记')
    ]
  },
  {
    name: 'roles',
    title: '角色与通用',
    items: [
      T('user', '用户'),
      T('users', '用户组'),
      T('id-badge-2', '账号'),
      T('star', '重点'),
      T('folder', '目录'),
      T('file-text', '文档'),
      T('bell', '告警'),
      T('clock', '时间'),
      T('search', '搜索'),
      T('settings', '配置'),
      T('chart-bar', '统计'),
      T('message-circle', '消息'),
      T('link', '关联')
    ]
  },
  {
    name: 'classic',
    title: '经典（写实）',
    items: [
      C('net/Router.png', '路由器(写实)'),
      C('ad/router.svg', '路由器(AD)'),
      C('net/Wireless_Router.png', '无线路由器(写实)'),
      C('net/Modem.png', '调制解调器(写实)'),
      C('net/Concentrator.png', '集中器(写实)'),
      C('net/Bridge.png', '网桥(写实)'),
      C('net/Repeater.png', '中继器(写实)'),
      C('net/Switch.png', '交换机(写实)'),
      C('ad/switch.svg', '交换机(AD)'),
      C('allied/Switch_24_port_L3.svg', '24口三层交换机'),
      C('allied/Switch_48_port_L3.svg', '48口交换机'),
      C('allied/Modular_Switch_SBx8106.svg', '框式交换机(实景)'),
      C('allied/Industrial_Ethernet_IE300.svg', '工业交换机(实景)'),
      C('allied/Access_Point_Indoor.svg', '室内AP(实景)'),
      C('allied/Access_Point_Outdoor.svg', '室外AP(实景)'),
      C('allied/Media_Converter_Standalone.svg', '光纤收发器(实景)'),
      C('net/Firewall.png', '防火墙(写实)'),
      C('net/Firewall_02.png', '防火墙(样式2)'),
      C('ad/firewall.svg', '防火墙(AD)'),
      C('allied/Router_VPN.svg', 'VPN路由器(实景)'),
      C('allied/Router_UTM.svg', 'UTM(实景)'),
      C('net/Certificate.png', '证书(写实)'),
      C('net/Certificate_Off.png', '证书(失效)'),
      C('net/Data_Filtering.png', '数据过滤'),
      C('ad/security.svg', '安全(AD)'),
      C('ad/key.svg', '密钥(AD)'),
      C('ad/tunnel.svg', '隧道(AD)'),
      C('net/Cloud.png', '云(写实)'),
      C('ad/internet_cloud.svg', '云(AD)'),
      C('net/Cloud_Server.png', '云服务器(写实)'),
      C('net/Cloud_Storage.png', '云存储(写实)'),
      C('net/Cloud_Computer.png', '云主机'),
      C('net/Cloud_Rack.png', '云机房'),
      C('net/Server.png', '服务器(写实)'),
      C('ad/generic_server.svg', '通用服务器(AD)'),
      C('ad/web_server.svg', 'Web服务器(AD)'),
      C('net/Database.png', '数据库(写实)'),
      C('ad/database_server.svg', '数据库服务器'),
      C('ad/sql_server.svg', 'SQL服务器(AD)'),
      C('net/Email.png', '邮件服务器(写实)'),
      C('net/Mainframe.png', '大型机(写实)'),
      C('ad/mainframe_host.svg', '主机(AD)'),
      C('ad/cluster_server.svg', '集群服务器(AD)'),
      C('ad/server_farm.svg', '服务器农场(AD)'),
      C('pc/Virtual_Machine.png', '虚拟机(写实)'),
      C('pc/Harddrive.png', '硬盘(写实)'),
      C('net/Server_Rack.png', '机架(写实)'),
      C('net/Server_Rack_Empty.png', '机柜(空)'),
      C('net/Server_Rack_Partial.png', '机柜(部分)'),
      C('allied/Datacenter_Server_Rack.svg', '数据中心机柜'),
      C('allied/Datacenter_Server_Rack_ToR.svg', 'ToR机柜'),
      C('allied/Datacenter_Server_Storage_Unit_Large.svg', '存储阵列(实景)'),
      C('net/UPS.png', 'UPS(写实)'),
      C('net/power_distribution_unit.png', 'PDU(写实)'),
      C('allied/Personal_Computer.svg', 'PC(实景)'),
      C('net/Print_Server.png', '打印服务器'),
      C('pc/Monitor_Tower.png', '台式机'),
      C('pc/Workstation.png', '工作站(写实)'),
      C('pc/Laptop.png', '笔记本(写实)'),
      C('pc/MacBook.png', 'MacBook'),
      C('pc/iMac.png', 'iMac'),
      C('pc/iPad.png', 'iPad'),
      C('allied/Tablet.svg', '平板(实景)'),
      C('allied/Smartphone.svg', '智能手机(实景)'),
      C('pc/Printer.png', '打印机(写实)'),
      C('net/Ip_Camera.png', 'IP摄像头(写实)'),
      C('ad/phone.svg', '电话(AD)'),
      C('ad/fax.svg', '传真(AD)'),
      C('ad/user.svg', '用户(AD)'),
      C('ad/users.svg', '用户组(AD)'),
      C('ad/generic_node.svg', '节点(AD)'),
      C('allied/Large_Building.svg', '大型建筑(实景)'),
      C('allied/Medium_Business_Building.svg', '商业楼宇(实景)'),
      C('allied/School_Building.svg', '学校(实景)'),
      C('allied/Secure_Building.svg', '安保建筑(实景)'),
      C('ad/home.svg', '家庭(AD)'),
      C('ad/data_jack.svg', '信息插座(AD)')
    ]
  }
]


/* 须在 initIcons() 完成后调用（保证 tabler 已测量、尺寸就绪）。
   尺寸档来自 iconCatalog（与 YAML 解析同源，手动拖放与 AI 导入视觉一致）。 */
export function buildCategories(): IconCategory[] {
  return CATEGORIES_DATA.map((cat) => ({
    name: cat.name,
    title: cat.title,
    items: cat.items.map((desc) => {
      if (desc.kind === 'c') {
        const k = scaleForClassicPath(desc.id)
        const asset = classicAsset(desc.id)
        if (asset) {
          if (isEndDeviceClassicPath(desc.id)) {
            const { w, h } = endFit(asset.w, asset.h)
            return { label: desc.label, src: asset.src, w, h }
          }
          const long = Math.max(asset.w, asset.h)
          const s = (76 * k) / long
          return { label: desc.label, src: asset.src, w: Math.round(asset.w * s), h: Math.round(asset.h * s) }
        }
        const w = Math.round(76 * k)
        return { label: desc.label, src: `/icons/${desc.id}`, w, h: w }
      }
      const s = iconSize(desc.id)
      if (isEndDevice(desc.id)) {
        const { w, h } = endFit(s.w, s.h)
        return { label: desc.label, src: iconUri(desc.id), w, h }
      }
      const k = scaleForModern(desc.id)
      return { label: desc.label, src: iconUri(desc.id), w: Math.round(s.w * k), h: Math.round(s.h * k) }
    })
  }))
}
