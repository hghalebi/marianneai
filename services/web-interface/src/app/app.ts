import {CommonModule, isPlatformBrowser} from '@angular/common';
import {ChangeDetectionStrategy, Component, inject, OnInit, PLATFORM_ID, signal} from '@angular/core';
import {FormsModule} from '@angular/forms';
import {firstValueFrom} from 'rxjs';
import {ApiService} from './api.service';
import {AnalyticsChart, QueryResponse, ReportArtifact} from './report.types';

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
  downloadingArtifact = signal<string | null>(null);
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
      const fallbackPdf = this.getReportArtifact('pdf');
      if (fallbackPdf?.download_url) {
        this.pdfErrorMessage.set('Le PDF premium est indisponible sur cet environnement. Téléchargement du PDF backend à la place.');
        await this.downloadArtifact(fallbackPdf);
      } else {
        this.pdfErrorMessage.set('Impossible de générer le PDF stylé pour le moment.');
      }
    } finally {
      this.isDownloadingPdf.set(false);
    }
  }

  async downloadArtifact(artifact: ReportArtifact) {
    if (!artifact.download_url || !isPlatformBrowser(this.platformId) || this.downloadingArtifact()) {
      return;
    }

    this.downloadingArtifact.set(artifact.filename);
    this.pdfErrorMessage.set(null);

    try {
      const response = await firstValueFrom(this.apiService.downloadReportArtifact(artifact));
      const fileName = this.extractFileName(response.headers.get('content-disposition')) || artifact.filename;
      const blob = response.body;

      if (!blob) {
        throw new Error('Fichier vide');
      }

      const objectUrl = window.URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = objectUrl;
      link.download = fileName;
      link.click();
      window.URL.revokeObjectURL(objectUrl);
    } catch (error) {
      console.error('Artifact download failed', error);
      this.pdfErrorMessage.set(`Impossible de télécharger ${artifact.filename} pour le moment.`);
    } finally {
      this.downloadingArtifact.set(null);
    }
  }

  getReportArtifact(format: string): ReportArtifact | undefined {
    return this.response()?.report_artifacts.find((artifact) => artifact.format === format);
  }

  hasBackendArtifact(format: string): boolean {
    return !!this.getReportArtifact(format)?.download_url;
  }

  getTopColumns(limit = 12): string[] {
    return (this.response()?.dataset_columns ?? []).slice(0, limit);
  }

  getCoverageLines(): string[] {
    const coverage = this.response()?.data_coverage ?? '';
    return coverage
      .split('|')
      .map((segment) => segment.trim())
      .filter(Boolean);
  }

  getPrintTimestamp(): string {
    return new Intl.DateTimeFormat('fr-FR', {
      dateStyle: 'long',
      timeStyle: 'short',
    }).format(new Date());
  }

  formatFieldLabel(value: string | null | undefined, maxLength = 48): string {
    if (!value) {
      return 'N/A';
    }
    const normalized = value
      .replace(/^\uFEFF/, '')
      .replace(/[_;]+/g, ' ')
      .replace(/\s+/g, ' ')
      .trim();
    if (!normalized) {
      return 'N/A';
    }
    return normalized.length > maxLength ? `${normalized.slice(0, maxLength - 1)}…` : normalized;
  }

  getChartColumnHeaders(chart: AnalyticsChart): string[] {
    return [chart.x_key, ...chart.y_keys];
  }

  getChartDataPreview(chart: AnalyticsChart): Array<Record<string, string | number | boolean | null>> {
    const maxPoints = chart.chart_type === 'scatter' ? 80 : 12;
    return chart.data.slice(0, maxPoints);
  }

  getChartDomain(values: number[]): {min: number; max: number} {
    if (values.length === 0) {
      return {min: 0, max: 1};
    }
    const min = Math.min(...values);
    const max = Math.max(...values);
    if (min === max) {
      return {min: min - 1, max: max + 1};
    }
    return {min, max};
  }

  getLineChartPoints(chart: AnalyticsChart): string {
    const yKey = chart.y_keys[0];
    if (!yKey) {
      return '';
    }
    const rows = this.getChartDataPreview(chart);
    const values = rows
      .map((row) => this.toNumber(row[yKey]))
      .filter((value): value is number => value !== null);
    if (rows.length === 0 || values.length === 0) {
      return '';
    }
    const {min, max} = this.getChartDomain(values);
    return rows
      .map((row, index) => {
        const value = this.toNumber(row[yKey]) ?? min;
        const x = rows.length === 1 ? 20 : 20 + (index * 260) / (rows.length - 1);
        const y = 120 - ((value - min) / (max - min || 1)) * 90;
        return `${x},${y}`;
      })
      .join(' ');
  }

  getBarChartBars(chart: AnalyticsChart): Array<{x: number; y: number; width: number; height: number; label: string; value: number}> {
    const yKey = chart.y_keys[0];
    if (!yKey) {
      return [];
    }
    const rows = this.getChartDataPreview(chart);
    const values = rows
      .map((row) => this.toNumber(row[yKey]))
      .filter((value): value is number => value !== null);
    if (rows.length === 0 || values.length === 0) {
      return [];
    }
    const max = Math.max(...values, 1);
    const width = Math.max(14, 220 / rows.length);
    return rows.map((row, index) => {
      const value = this.toNumber(row[yKey]) ?? 0;
      const height = (value / max) * 90;
      return {
        x: 25 + index * (width + 8),
        y: 120 - height,
        width,
        height,
        label: String(row[chart.x_key] ?? `#${index + 1}`),
        value,
      };
    });
  }

  getScatterChartPoints(chart: AnalyticsChart): Array<{cx: number; cy: number; label: string}> {
    const yKey = chart.y_keys[0];
    if (!yKey) {
      return [];
    }
    const rows = this.getChartDataPreview(chart);
    const xValues = rows.map((row) => this.toNumber(row[chart.x_key])).filter((value): value is number => value !== null);
    const yValues = rows.map((row) => this.toNumber(row[yKey])).filter((value): value is number => value !== null);
    if (rows.length === 0 || xValues.length === 0 || yValues.length === 0) {
      return [];
    }
    const xDomain = this.getChartDomain(xValues);
    const yDomain = this.getChartDomain(yValues);
    return rows
      .map((row) => {
        const xValue = this.toNumber(row[chart.x_key]);
        const yValue = this.toNumber(row[yKey]);
        if (xValue === null || yValue === null) {
          return null;
        }
        return {
          cx: 20 + ((xValue - xDomain.min) / (xDomain.max - xDomain.min || 1)) * 260,
          cy: 120 - ((yValue - yDomain.min) / (yDomain.max - yDomain.min || 1)) * 90,
          label: `${this.formatFieldLabel(chart.x_key)}: ${xValue} | ${this.formatFieldLabel(yKey)}: ${yValue}`,
        };
      })
      .filter((point): point is {cx: number; cy: number; label: string} => point !== null);
  }

  formatNumber(value: number | null | undefined): string {
    if (value === null || value === undefined || Number.isNaN(value)) {
      return 'N/A';
    }
    return new Intl.NumberFormat('fr-FR', {
      maximumFractionDigits: Math.abs(value) >= 100 ? 0 : 2,
    }).format(value);
  }

  formatPercent(value: number): string {
    return new Intl.NumberFormat('fr-FR', {
      style: 'percent',
      minimumFractionDigits: 0,
      maximumFractionDigits: 1,
    }).format(value);
  }

  private toNumber(value: string | number | boolean | null | undefined): number | null {
    if (typeof value === 'number') {
      return Number.isFinite(value) ? value : null;
    }
    if (typeof value === 'boolean') {
      return value ? 1 : 0;
    }
    if (typeof value === 'string' && value.trim() !== '') {
      const parsed = Number(value.replace(',', '.'));
      return Number.isFinite(parsed) ? parsed : null;
    }
    return null;
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
