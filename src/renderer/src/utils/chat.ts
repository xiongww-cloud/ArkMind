export const RETRIEVAL_PHRASES = [
  '正在检索知识库，请稍等…',
  '让我翻翻资料…',
  '我在查找相关内容…',
  '稍等，正在定位信息…',
  '检索中，马上就好…',
  '正在知识库里翻找，别急…',
  '让我查一下相关资料…',
  '资料检索中，请稍候…',
]

export function pickRetrievalPhrase(): string {
  return RETRIEVAL_PHRASES[Math.floor(Math.random() * RETRIEVAL_PHRASES.length)]
}
