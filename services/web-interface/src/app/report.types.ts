export interface Source {
  title: string;
  url: string;
  description: string;
  reason_for_selection: string;
  confidence_score: number;
}

export interface UsedResource {
  dataset_title: string;
  resource_title: string;
  resource_url: string;
  format: string;
}

export interface DescriptiveStatistic {
  column: string;
  non_null_count: number;
  mean: number | null;
  min: number | null;
  max: number | null;
  median: number | null;
  stddev: number | null;
}

export interface RegressionResult {
  feature_x: string;
  feature_y: string;
  slope: number;
  intercept: number;
  r_squared: number;
  sample_size: number;
}

export interface AnalyticsChart {
  chart_id: string;
  title: string;
  chart_type: string;
  description: string;
  x_key: string;
  y_keys: string[];
  data: Array<Record<string, string | number | boolean | null>>;
}

export interface ReportArtifact {
  report_id: string;
  format: string;
  filename: string;
  download_url: string;
  content_type: string;
}

export interface QueryResponse {
  user_query: string;
  selected_sources: Source[];
  answer: string;
  limitations: string[];
  trace: string[];
  analysis_engine: string;
  analysis_summary: string;
  key_findings: string[];
  data_coverage: string;
  dataset_row_count: number | null;
  dataset_columns: string[];
  descriptive_statistics: DescriptiveStatistic[];
  regressions: RegressionResult[];
  charts: AnalyticsChart[];
  used_resources: UsedResource[];
  report_artifacts: ReportArtifact[];
}
