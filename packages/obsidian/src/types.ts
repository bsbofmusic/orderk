
export type OrderkSearchResult = {
  path: string;
  title?: string;
  snippet?: string;
  score?: number;
  line_start?: number;
  line_end?: number;
  heading?: string;
  chunk_id?: string;
};

export type OrderkFeedbackEvent = {
  type: "search" | "open" | "dismiss";
  query?: string;
  path?: string;
  rank?: number;
  timestamp: string;
};
