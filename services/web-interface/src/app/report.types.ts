export interface Source {
  title: string;
  url: string;
  description: string;
  reason_for_selection: string;
  confidence_score: number;
}

export interface QueryResponse {
  user_query: string;
  selected_sources: Source[];
  answer: string;
  limitations: string[];
  trace: string[];
}
