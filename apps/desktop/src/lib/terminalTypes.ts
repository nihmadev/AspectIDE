export type TerminalOutputBuffer = {
  text: string;
  updatedAt: string | null;
  bytes: number;
  chunks: number;
  truncated: boolean;
};
