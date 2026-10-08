// Local text validation only. Native planning validation remains authoritative.
export const MAX_COMMAND_BYTES = 8192;
export function utf8Bytes(text: string): number { return new TextEncoder().encode(text).length; }
export function transcriptHandoffError(text: string): string | null {
  if (!text.trim()) return "The transcript is empty. Nothing was copied to Ask GHOST.";
  if (utf8Bytes(text) > MAX_COMMAND_BYTES) return "The transcript exceeds the 8,192 UTF-8 byte limit. Nothing was copied or truncated; the original transcript remains available.";
  return null;
}
export interface TranscriptTarget { available: boolean; busy: boolean; replacesDraft: boolean }
