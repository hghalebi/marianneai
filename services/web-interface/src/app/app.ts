import {CommonModule, isPlatformBrowser} from '@angular/common';
import {ChangeDetectionStrategy, Component, inject, OnInit, PLATFORM_ID, signal} from '@angular/core';
import {FormsModule} from '@angular/forms';
import {ApiService, QueryResponse} from './api.service';

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

    try {
      const {jsPDF} = await import('jspdf');
      const pdf = new jsPDF({format: 'a4', unit: 'mm'});
      const margin = 16;
      const pageHeight = pdf.internal.pageSize.getHeight();
      const contentWidth = pdf.internal.pageSize.getWidth() - margin * 2;
      const lineHeight = 6;
      let cursorY = margin;

      const ensureSpace = (requiredHeight = lineHeight) => {
        if (cursorY + requiredHeight <= pageHeight - margin) {
          return;
        }

        pdf.addPage();
        cursorY = margin;
      };

      const addParagraph = (text: string, indent = 0) => {
        const lines = pdf.splitTextToSize(text, contentWidth - indent);
        ensureSpace(lines.length * lineHeight + 2);
        pdf.setFont('helvetica', 'normal');
        pdf.setFontSize(11);
        pdf.text(lines, margin + indent, cursorY);
        cursorY += lines.length * lineHeight + 2;
      };

      const addSectionTitle = (text: string) => {
        ensureSpace(10);
        pdf.setFont('helvetica', 'bold');
        pdf.setFontSize(13);
        pdf.text(text, margin, cursorY);
        cursorY += 8;
      };

      const addBulletList = (items: string[]) => {
        items.forEach((item) => addParagraph(`• ${item}`, 2));
        cursorY += 1;
      };

      pdf.setFont('helvetica', 'bold');
      pdf.setFontSize(18);
      pdf.text('Rapport MarianneAI', margin, cursorY);
      cursorY += 10;

      addParagraph(`Généré le ${new Intl.DateTimeFormat('fr-FR', {
        dateStyle: 'full',
        timeStyle: 'short',
      }).format(new Date())}`);

      addSectionTitle('Question');
      addParagraph(report.user_query);

      addSectionTitle('Réponse');
      addParagraph(report.answer);

      if (report.selected_sources.length > 0) {
        addSectionTitle('Sources');

        report.selected_sources.forEach((source, index) => {
          addParagraph(`${index + 1}. ${source.title}`);
          addParagraph(`URL : ${source.url}`, 4);
          addParagraph(source.description, 4);
          addParagraph(`Pourquoi cette source : ${source.reason_for_selection}`, 4);
          addParagraph(`Score de confiance : ${Math.round(source.confidence_score * 100)} %`, 4);
          cursorY += 1;
        });
      }

      if (report.limitations.length > 0) {
        addSectionTitle('Limites');
        addBulletList(report.limitations);
      }

      if (report.trace.length > 0) {
        addSectionTitle('Trace');
        addBulletList(report.trace.map((step, index) => `${index + 1}. ${step}`));
      }

      const fileDate = new Intl.DateTimeFormat('fr-CA', {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit',
      }).format(new Date()).replaceAll('/', '-');

      pdf.save(`rapport-marianneai-${fileDate}.pdf`);
    } finally {
      this.isDownloadingPdf.set(false);
    }
  }
}
