export interface IconItem {
  label: string
  src: string
}

export interface IconCategory {
  name: string
  title: string
  items: IconItem[]
}

export const CATEGORIES: IconCategory[] = [
  {
    name: 'routing',
    title: '路由与交换',
    items: [
      { label: '路由器', src: '/icons/net/Router.png' },
      { label: '路由器(AD)', src: '/icons/ad/router.svg' },
      { label: '无线路由器', src: '/icons/net/Wireless_Router.png' },
      { label: '调制解调器', src: '/icons/net/Modem.png' },
      { label: '集中器', src: '/icons/net/Concentrator.png' },
      { label: '网桥', src: '/icons/net/Bridge.png' },
      { label: '中继器', src: '/icons/net/Repeater.png' },
      { label: '交换机', src: '/icons/net/Switch.png' },
      { label: '交换机(AD)', src: '/icons/ad/switch.svg' },
      { label: '三层交换机', src: '/icons/allied/Switch_24_port_L3.svg' },
      { label: '48口交换机', src: '/icons/allied/Switch_48_port_L3.svg' },
      { label: '框式交换机', src: '/icons/allied/Modular_Switch_SBx8106.svg' },
      { label: '工业交换机', src: '/icons/allied/Industrial_Ethernet_IE300.svg' },
      { label: '光纤收发器', src: '/icons/allied/Media_Converter_Standalone.svg' },
      { label: '室内AP', src: '/icons/allied/Access_Point_Indoor.svg' },
      { label: '室外AP', src: '/icons/allied/Access_Point_Outdoor.svg' }
    ]
  },
  {
    name: 'security',
    title: '安全',
    items: [
      { label: '防火墙', src: '/icons/net/Firewall.png' },
      { label: '防火墙(AD)', src: '/icons/ad/firewall.svg' },
      { label: '防火墙(样式2)', src: '/icons/net/Firewall_02.png' },
      { label: 'VPN路由器', src: '/icons/allied/Router_VPN.svg' },
      { label: 'UTM', src: '/icons/allied/Router_UTM.svg' },
      { label: '证书', src: '/icons/net/Certificate.png' },
      { label: '证书(失效)', src: '/icons/net/Certificate_Off.png' },
      { label: '数据过滤', src: '/icons/net/Data_Filtering.png' },
      { label: '安全', src: '/icons/ad/security.svg' },
      { label: '密钥', src: '/icons/ad/key.svg' },
      { label: '隧道', src: '/icons/ad/tunnel.svg' }
    ]
  },
  {
    name: 'cloud',
    title: '云与互联网',
    items: [
      { label: '云', src: '/icons/net/Cloud.png' },
      { label: '云(AD)', src: '/icons/ad/internet_cloud.svg' },
      { label: '互联网', src: '/icons/ad/internet_globe.svg' },
      { label: '云服务器', src: '/icons/net/Cloud_Server.png' },
      { label: '云存储', src: '/icons/net/Cloud_Storage.png' },
      { label: '云主机', src: '/icons/net/Cloud_Computer.png' },
      { label: '云机房', src: '/icons/net/Cloud_Rack.png' }
    ]
  },
  {
    name: 'server',
    title: '服务器与存储',
    items: [
      { label: '服务器', src: '/icons/net/Server.png' },
      { label: '通用服务器', src: '/icons/ad/generic_server.svg' },
      { label: 'Web服务器', src: '/icons/ad/web_server.svg' },
      { label: '数据库', src: '/icons/net/Database.png' },
      { label: '数据库服务器', src: '/icons/ad/database_server.svg' },
      { label: 'SQL服务器', src: '/icons/ad/sql_server.svg' },
      { label: '邮件服务器', src: '/icons/net/Email.png' },
      { label: '大型机', src: '/icons/net/Mainframe.png' },
      { label: '主机', src: '/icons/ad/mainframe_host.svg' },
      { label: '集群服务器', src: '/icons/ad/cluster_server.svg' },
      { label: '服务器农场', src: '/icons/ad/server_farm.svg' },
      { label: '虚拟机', src: '/icons/pc/Virtual_Machine.png' },
      { label: '硬盘', src: '/icons/pc/Harddrive.png' },
      { label: '机架', src: '/icons/net/Server_Rack.png' },
      { label: '机柜(空)', src: '/icons/net/Server_Rack_Empty.png' },
      { label: '数据中心机柜', src: '/icons/allied/Datacenter_Server_Rack.svg' },
      { label: 'ToR机柜', src: '/icons/allied/Datacenter_Server_Rack_ToR.svg' },
      { label: '存储阵列', src: '/icons/allied/Datacenter_Server_Storage_Unit_Large.svg' },
      { label: 'UPS', src: '/icons/net/UPS.png' },
      { label: 'PDU', src: '/icons/net/power_distribution_unit.png' }
    ]
  },
  {
    name: 'terminal',
    title: '终端与办公',
    items: [
      { label: 'PC', src: '/icons/allied/Personal_Computer.svg' },
      { label: '台式机', src: '/icons/pc/Monitor_Tower.png' },
      { label: '工作站', src: '/icons/pc/Workstation.png' },
      { label: '笔记本', src: '/icons/pc/Laptop.png' },
      { label: 'MacBook', src: '/icons/pc/MacBook.png' },
      { label: 'iMac', src: '/icons/pc/iMac.png' },
      { label: 'iPad', src: '/icons/pc/iPad.png' },
      { label: '平板', src: '/icons/allied/Tablet.svg' },
      { label: '智能手机', src: '/icons/allied/Smartphone.svg' },
      { label: '打印机', src: '/icons/pc/Printer.png' },
      { label: '打印服务器', src: '/icons/net/Print_Server.png' },
      { label: 'IP摄像头', src: '/icons/net/Ip_Camera.png' },
      { label: '电话', src: '/icons/ad/phone.svg' },
      { label: '传真', src: '/icons/ad/fax.svg' },
      { label: '用户', src: '/icons/ad/user.svg' },
      { label: '用户组', src: '/icons/ad/users.svg' },
      { label: '节点', src: '/icons/ad/generic_node.svg' }
    ]
  },
  {
    name: 'site',
    title: '建筑与场地',
    items: [
      { label: '大型建筑', src: '/icons/allied/Large_Building.svg' },
      { label: '商业楼宇', src: '/icons/allied/Medium_Business_Building.svg' },
      { label: '学校', src: '/icons/allied/School_Building.svg' },
      { label: '安保建筑', src: '/icons/allied/Secure_Building.svg' },
      { label: '家庭', src: '/icons/ad/home.svg' },
      { label: '信息插座', src: '/icons/ad/data_jack.svg' }
    ]
  }
]
