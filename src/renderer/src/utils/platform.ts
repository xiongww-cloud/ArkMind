/** 是否 macOS（WebKit：不把剪贴板文件暴露给 clipboardData，需后端 NSPasteboard 回退） */
export const isMac: boolean =
  typeof navigator !== 'undefined' &&
  (navigator.platform.toUpperCase().indexOf('MAC') >= 0 ||
    navigator.userAgent.toUpperCase().indexOf('MAC') >= 0)
