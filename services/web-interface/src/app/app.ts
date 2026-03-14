import {CommonModule, isPlatformBrowser} from '@angular/common';
import {ChangeDetectionStrategy, Component, inject, OnInit, PLATFORM_ID, signal} from '@angular/core';
import {FormsModule} from '@angular/forms';
import {firstValueFrom} from 'rxjs';
import {ApiService} from './api.service';
import {QueryResponse} from './report.types';

@Component({
  changeDetection: ChangeDetectionStrategy.OnPush,
  selector: 'app-root',
  imports: [FormsModule, CommonModule],
  templateUrl: './app.html',
  styleUrl: './app.css',
})
export class App implements OnInit {
  apiService = inject(ApiService);
  private readonly platformId = inject(PLATFORM_ID);

  queryText = signal('');
  isLoading = signal(false);
  isDownloadingPdf = signal(false);
  pdfErrorMessage = signal<string | null>(null);
  response = signal<QueryResponse | null>(null);
  scenarios = signal<string[]>([]);

  loadingMessages = [
    'Analyse de votre demande...',
    'Recherche sur data.gouv.fr...',
    'Filtrage des jeux de données pertinents...',
    'Synthèse de la réponse...',
  ];
  currentLoadingMessageIndex = signal(0);
  loadingInterval: ReturnType<typeof setInterval> | undefined;

  ngOnInit() {
    if (!isPlatformBrowser(this.platformId)) {
      return;
    }

    this.apiService.getScenarios().subscribe((res) => {
      this.scenarios.set(res.scenarios);
    });
  }

  selectScenario(scenario: string) {
    this.queryText.set(scenario);
    this.submitQuery();
  }

  submitQuery() {
    const text = this.queryText().trim();
    if (!text) return;

    this.isLoading.set(true);
    this.response.set(null);
    this.pdfErrorMessage.set(null);
    this.currentLoadingMessageIndex.set(0);

    this.loadingInterval = setInterval(() => {
      this.currentLoadingMessageIndex.update((i) => (i + 1) % this.loadingMessages.length);
    }, 2000);

    this.apiService.query(text).subscribe({
      next: (res) => {
        clearInterval(this.loadingInterval);
        this.response.set(res);
        this.isLoading.set(false);
      },
      error: () => {
        clearInterval(this.loadingInterval);
        this.isLoading.set(false);
      }
    });
  }

  async downloadPdf() {
    const report = this.response();
    if (!report || !isPlatformBrowser(this.platformId) || this.isDownloadingPdf()) {
      return;
    }

    this.isDownloadingPdf.set(true);
    this.pdfErrorMessage.set(null);

    try {
      const response = await firstValueFrom(this.apiService.downloadStyledPdf(report));
      const fileName = this.extractFileName(response.headers.get('content-disposition'));
      const pdfBlob = response.body;

      if (!pdfBlob) {
        throw new Error('PDF vide');
      }

      const objectUrl = window.URL.createObjectURL(pdfBlob);
      const link = document.createElement('a');
      link.href = objectUrl;
      link.download = fileName;
      link.click();
      window.URL.revokeObjectURL(objectUrl);
    } catch (error) {
      console.error('Styled PDF generation failed', error);
      this.pdfErrorMessage.set('Impossible de générer le PDF stylé pour le moment.');
    } finally {
      this.isDownloadingPdf.set(false);
    }
  }

  private extractFileName(contentDisposition: string | null): string {
    if (!contentDisposition) {
      return this.buildDefaultPdfFileName();
    }

    const match = /filename="([^"]+)"/i.exec(contentDisposition);
    return match?.[1] ?? this.buildDefaultPdfFileName();
  }

  private buildDefaultPdfFileName(): string {
    const fileDate = new Intl.DateTimeFormat('fr-CA', {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
    }).format(new Date()).replaceAll('/', '-');

    return `rapport-marianneai-style-${fileDate}.pdf`;
  }
}
