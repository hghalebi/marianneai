import {HttpClient, HttpResponse} from '@angular/common/http';
import {Injectable, inject, signal} from '@angular/core';
import {Observable, catchError, of, tap} from 'rxjs';
import {QueryResponse} from './report.types';

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
      tap(() => this.isMockMode.set(false)),
      catchError((err) => {
        this.isMockMode.set(true);
        console.warn('API call failed (CORS/Mixed Content or Offline). Using mock data.', err.message);
        return of({
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
          trace: [
            'Analyse de la demande utilisateur.',
            'Orchestrateur : préparation de la recherche.',
            'Recherche des jeux de données sur data.gouv.fr.',
            'Filtrage des jeux de données retournés.',
            'Sélection des sources les plus pertinentes.',
            'Synthèse de la réponse finale.',
            'Rapport prêt.'
          ]
        });
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
}
