import {HttpClient, HttpResponse} from '@angular/common/http';
import {Injectable, inject, signal} from '@angular/core';
import {Observable, catchError, map, of, tap} from 'rxjs';
import {QueryResponse, ReportArtifact} from './report.types';

export interface ScenariosResponse {
  scenarios: string[];
}

@Injectable({providedIn: 'root'})
export class ApiService {
  private http = inject(HttpClient);

  private readonly queryEndpoint = '/api/query';
  private readonly scenariosEndpoint = '/api/demo/scenarios';
  private readonly reportPdfEndpoint = '/api/report/pdf';
  isMockMode = signal(false);

  query(text: string): Observable<QueryResponse> {
    return this.http.post<QueryResponse>(this.queryEndpoint, {query: text}).pipe(
      map((response) => this.normalizeQueryResponse(response)),
      tap(() => this.isMockMode.set(false)),
      catchError((err) => {
        this.isMockMode.set(true);
        console.warn('API call failed (CORS/Mixed Content or Offline). Using mock data.', err.message);
        return of(this.normalizeQueryResponse({
          user_query: text,
          selected_sources: [
            {
              title: 'Fichier consolidé des bornes de recharge (IRVE)',
              url: 'https://www.data.gouv.fr/fr/datasets/fichier-consolide-...',
              description: 'Base de données consolidée des infrastructures de recharge publique.',
              reason_for_selection: 'Répond directement à la requête avec des données officielles et consolidées.',
              confidence_score: 0.95
            }
          ],
          answer:
            "D'après data.gouv.fr, le jeu de données le plus pertinent est le 'Fichier consolidé des bornes de recharge pour véhicules électriques (IRVE)'. Il recense l'ensemble des infrastructures publiques en France.",
          limitations: [
            "Le jeu de données ne couvre que les bornes publiques, les bornes résidentielles privées n'y figurent pas."
          ],
          analysis_engine: 'mock-demo',
          analysis_summary:
            "Analyse de démonstration générée localement à partir d'un scénario mock pour garder l'interface fonctionnelle.",
          key_findings: [
            "Le backend a identifié un jeu de données officiel pertinent sur les bornes de recharge.",
            "Un rapport PDF et un rapport Excel peuvent être proposés dans l'expérience réelle.",
          ],
          data_coverage: 'Couverture estimative France entière, données publiques, détails indisponibles en mode mock.',
          dataset_row_count: 2500,
          dataset_columns: ['date', 'region', 'stations', 'charge_points'],
          descriptive_statistics: [
            {
              column: 'charge_points',
              non_null_count: 2500,
              mean: 12.8,
              min: 1,
              max: 48,
              median: 8,
              stddev: 7.1,
            },
          ],
          regressions: [
            {
              feature_x: 'stations',
              feature_y: 'charge_points',
              slope: 2.31,
              intercept: 0.42,
              r_squared: 0.91,
              sample_size: 2500,
            },
          ],
          charts: [
            {
              chart_id: 'mock-time-series',
              title: 'Points de charge dans le temps',
              chart_type: 'line',
              description: 'Evolution simulée du nombre moyen de points de charge.',
              x_key: 'date',
              y_keys: ['charge_points'],
              data: [
                {date: '2025-01-01', charge_points: 9},
                {date: '2025-03-01', charge_points: 11},
                {date: '2025-06-01', charge_points: 13},
                {date: '2025-09-01', charge_points: 14},
                {date: '2025-12-01', charge_points: 16},
              ],
            },
          ],
          used_resources: [
            {
              dataset_title: 'Fichier consolidé des bornes de recharge (IRVE)',
              resource_title: 'irve-national-summary.csv',
              resource_url: '/api/demo/scenarios',
              format: 'csv',
            },
          ],
          report_artifacts: [
            {
              report_id: 'mock-report',
              format: 'pdf',
              filename: 'analysis-report.pdf',
              download_url: '/api/report/pdf',
              content_type: 'application/pdf',
            },
            {
              report_id: 'mock-report',
              format: 'xlsx',
              filename: 'analysis-report.xlsx',
              download_url: '',
              content_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
            },
          ],
          trace: [
            'Analyse de la demande utilisateur.',
            'Orchestrateur : préparation de la recherche.',
            'Recherche des jeux de données sur data.gouv.fr.',
            'Filtrage des jeux de données retournés.',
            'Sélection des sources les plus pertinentes.',
            'Synthèse de la réponse finale.',
            'Rapport prêt.'
          ]
        }));
      })
    );
  }

  getScenarios(): Observable<ScenariosResponse> {
    return this.http.get<ScenariosResponse>(this.scenariosEndpoint).pipe(
      tap(() => this.isMockMode.set(false)),
      catchError((err) => {
        this.isMockMode.set(true);
        console.warn('API call failed. Using mock data.', err.message);
        return of({
          scenarios: [
            'Quels sont les derniers jeux de données sur les bornes de recharge électrique en France ?',
            "Trouve des données sur la qualité de l'eau à Paris.",
            'Y a-t-il des jeux de données concernant les retards des transports en commun à Lyon ?'
          ]
        });
      })
    );
  }

  downloadStyledPdf(report: QueryResponse): Observable<HttpResponse<Blob>> {
    return this.http.post(this.reportPdfEndpoint, report, {
      observe: 'response',
      responseType: 'blob',
    });
  }

  downloadReportArtifact(artifact: ReportArtifact): Observable<HttpResponse<Blob>> {
    return this.http.get(artifact.download_url, {
      observe: 'response',
      responseType: 'blob',
    });
  }

  private normalizeQueryResponse(response: QueryResponse): QueryResponse {
    const reportArtifacts = (response.report_artifacts ?? []).map((artifact) => ({
      ...artifact,
      download_url: this.normalizeDownloadUrl(artifact.download_url),
    }));

    return {
      ...response,
      analysis_engine: response.analysis_engine ?? 'unknown',
      analysis_summary: response.analysis_summary ?? '',
      key_findings: response.key_findings ?? [],
      data_coverage: response.data_coverage ?? '',
      dataset_row_count: response.dataset_row_count ?? null,
      dataset_columns: response.dataset_columns ?? [],
      descriptive_statistics: response.descriptive_statistics ?? [],
      regressions: response.regressions ?? [],
      charts: response.charts ?? [],
      used_resources: response.used_resources ?? [],
      report_artifacts: reportArtifacts,
    };
  }

  private normalizeDownloadUrl(downloadUrl: string): string {
    if (!downloadUrl) {
      return '';
    }
    if (downloadUrl.startsWith('/api/')) {
      return downloadUrl;
    }
    if (downloadUrl.startsWith('/')) {
      return `/api${downloadUrl}`;
    }
    return `/api/${downloadUrl}`;
  }
}
