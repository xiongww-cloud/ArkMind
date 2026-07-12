import { describe, it, expect } from 'vitest'
import { formatDateTime, formatDate, formatRelativeTime } from '../utils/time'

describe('time utils', () => {
  describe('formatDateTime', () => {
    it('returns empty string for null/undefined', () => {
      expect(formatDateTime(null)).toBe('')
      expect(formatDateTime(undefined)).toBe('')
    })

    it('formats a valid ISO string', () => {
      const result = formatDateTime('2026-06-07T19:55:00')
      expect(result).toBe('2026-06-07 19:55')
    })

    it('returns empty string for invalid date', () => {
      expect(formatDateTime('not-a-date')).toBe('')
    })
  })

  describe('formatDate', () => {
    it('returns empty string for null/undefined', () => {
      expect(formatDate(null)).toBe('')
      expect(formatDate(undefined)).toBe('')
    })

    it('formats a valid ISO string', () => {
      const result = formatDate('2026-06-07T19:55:00')
      expect(result).toBe('2026/06/07')
    })

    it('returns empty string for invalid date', () => {
      expect(formatDate('invalid')).toBe('')
    })
  })

  describe('formatRelativeTime', () => {
    it('returns empty string for null/undefined', () => {
      expect(formatRelativeTime(null)).toBe('')
      expect(formatRelativeTime(undefined)).toBe('')
    })

    it('returns empty string for invalid date', () => {
      expect(formatRelativeTime('invalid')).toBe('')
    })

    it('returns relative time for recent dates', () => {
      const now = new Date()
      const fiveMinutesAgo = new Date(now.getTime() - 5 * 60 * 1000)
      const result = formatRelativeTime(fiveMinutesAgo.toISOString())
      expect(result).toBe('5 分钟前')
    })
  })
})