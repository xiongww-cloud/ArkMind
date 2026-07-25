const retrievalPhrases = [
  '正在分析您的问题...',
  '让我想想...',
  '正在思考...',
  '检索中...',
  '搜索相关知识...',
  '让我查找一下...',
  '正在整理信息...',
  '处理中...',
  '分析数据中...',
  '查找相关资料...',
  '思考中...',
  '正在处理...',
]

export function pickRetrievalPhrase(): string {
  return retrievalPhrases[Math.floor(Math.random() * retrievalPhrases.length)]
}
