import {GoogleGenAI} from '@google/genai';
import {spawn} from 'node:child_process';
import {copyFile, mkdtemp, readFile, rm, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {QueryResponse} from '../app/report.types';

const BRAND_COLORS = {
  blue: '#002654',
  red: '#ED2939',
  white: '#FFFFFF',
};

const GEMINI_PLACEHOLDERS = new Set([
  '',
  'YOUR_GEMINI_API_KEY',
  'MY_GEMINI_API_KEY',
]);

const ALLOWED_LATEX_COMMANDS = new Set([
  'reportsubtitle',
  'highlightbox',
  'sectiontitle',
  'bodytext',
  'sourcecard',
  'bulletitem',
  'traceitem',
]);

const FORBIDDEN_LATEX_PATTERNS = [
  '\\documentclass',
  '\\begin{document}',
  '\\end{document}',
  '\\usepackage',
  '\\input',
  '\\include',
  '\\write18',
  '\\openout',
  '\\openin',
  '\\read',
  '\\catcode',
  '\\newcommand',
  '\\renewcommand',
  '\\immediate',
];

export interface StyledPdfResult {
  fileName: string;
  pdfBuffer: Buffer;
  source: 'gemini' | 'fallback';
}

export async function createStyledPdf(report: QueryResponse, logoPath: string): Promise<StyledPdfResult> {
  const tempDirectory = await mkdtemp(join(tmpdir(), 'marianneai-pdf-'));
  const fileName = buildPdfFileName();

  try {
    await copyFile(logoPath, join(tempDirectory, 'logo-mariene.png'));

    let latexFragment = buildFallbackLatexFragment(report);
    let source: StyledPdfResult['source'] = 'fallback';

    if (isGeminiConfigured(process.env['GEMINI_API_KEY']) && process.env['ENABLE_GEMINI_PDF_STYLING'] === 'true') {
      try {
        const geminiFragment = await buildGeminiLatexFragment(report);
        if (isSafeLatexFragment(geminiFragment)) {
          latexFragment = geminiFragment;
          source = 'gemini';
        }
      } catch (error) {
        console.error('Gemini LaTeX generation failed, using fallback template', error);
      }
    }

    const latexDocument = buildLatexDocument(latexFragment);
    const texFileName = 'rapport.tex';
    await writeFile(join(tempDirectory, texFileName), latexDocument, 'utf8');

    try {
      await runPdflatex(tempDirectory, texFileName);
    } catch (error) {
      if (source === 'gemini') {
        const fallbackDocument = buildLatexDocument(buildFallbackLatexFragment(report));
        await writeFile(join(tempDirectory, texFileName), fallbackDocument, 'utf8');
        await runPdflatex(tempDirectory, texFileName);
        source = 'fallback';
      } else {
        throw error;
      }
    }

    const pdfBuffer = await readFile(join(tempDirectory, 'rapport.pdf'));
    return {fileName, pdfBuffer, source};
  } finally {
    await rm(tempDirectory, {recursive: true, force: true});
  }
}

function isGeminiConfigured(apiKey: string | undefined): boolean {
  return !GEMINI_PLACEHOLDERS.has((apiKey ?? '').trim());
}

async function buildGeminiLatexFragment(report: QueryResponse): Promise<string> {
  const ai = new GoogleGenAI({apiKey: process.env['GEMINI_API_KEY']});
  const response = await ai.models.generateContent({
    model: process.env['GEMINI_PDF_MODEL'] || 'gemini-2.5-flash',
    contents: buildGeminiPrompt(report),
  });

  return cleanLatexFragment(response.text ?? '');
}

function buildGeminiPrompt(report: QueryResponse): string {
  return [
    'Tu es un directeur artistique qui prépare un fragment LaTeX élégant pour un PDF MarianneAI.',
    'Le backend ajoute déjà le logo logo-mariene.png dans l’entête et compile le document.',
    'Tu dois seulement retourner un fragment LaTeX sans préambule ni ```.',
    'Couleurs de marque :',
    `- Bleu MarianneAI : ${BRAND_COLORS.blue}`,
    `- Rouge MarianneAI : ${BRAND_COLORS.red}`,
    `- Blanc : ${BRAND_COLORS.white}`,
    'Tu dois utiliser exclusivement les macros suivantes :',
    '\\reportsubtitle{...}',
    '\\highlightbox{titre}{texte}',
    '\\sectiontitle{...}',
    '\\bodytext{...}',
    '\\sourcecard{titre}{url}{description}{raison}{scorePourcent}',
    '\\bulletitem{...}',
    '\\traceitem{label}{...}',
    'Contraintes :',
    '- N’utilise aucune autre commande LaTeX.',
    '- N’invente aucun fait.',
    '- Garde un ton clair, citoyen, premium et institutionnel.',
    '- Résume intelligemment la réponse.',
    '- Garde les sources, les limites et la trace si elles sont présentes.',
    '- Échappe les caractères spéciaux LaTeX dans les arguments.',
    '- Retourne uniquement le fragment.',
    'Données du rapport :',
    JSON.stringify(report, null, 2),
  ].join('\n');
}

function cleanLatexFragment(fragment: string): string {
  return fragment
    .replace(/^```(?:latex)?/i, '')
    .replace(/```$/i, '')
    .trim();
}

function isSafeLatexFragment(fragment: string): boolean {
  if (!fragment) {
    return false;
  }

  const normalizedFragment = fragment.toLowerCase();
  if (FORBIDDEN_LATEX_PATTERNS.some((pattern) => normalizedFragment.includes(pattern.toLowerCase()))) {
    return false;
  }

  const commands = [...fragment.matchAll(/\\([a-zA-Z]+)/g)].map((match) => match[1]);
  return commands.every((command) => ALLOWED_LATEX_COMMANDS.has(command));
}

function buildFallbackLatexFragment(report: QueryResponse): string {
  const metricsLine = [
    `\\metriccard{Moteur d'analyse}{${escapeLatex(report.analysis_engine || 'N/A')}}`,
    `\\metriccard{Lignes analysées}{${escapeLatex(formatMetric(report.dataset_row_count))}}`,
    `\\metriccard{Ressources exploitées}{${escapeLatex(String(report.used_resources.length))}}`,
  ].join('\n\\hfill\n');

  const lines = [
    '\\reportsubtitle{Synthèse citoyenne premium générée automatiquement pour la démo MarianneAI.}',
    `\\highlightbox{Résumé exécutif}{${escapeLatex(report.answer)}}`,
    '\\sectiontitle{Instantané}',
    metricsLine,
    '\\sectiontitle{Question posée}',
    `\\bodytext{${escapeLatex(report.user_query)}}`,
  ];

  if (report.analysis_summary) {
    lines.push('\\sectiontitle{Lecture analytique}');
    lines.push(`\\highlightbox{Synthèse analytique}{${escapeLatex(report.analysis_summary)}}`);
  }

  if (report.key_findings.length > 0) {
    lines.push('\\sectiontitle{Constats clés}');
    report.key_findings.forEach((finding) => {
      lines.push(`\\bulletitem{${escapeLatex(finding)}}`);
    });
  }

  if (report.data_coverage) {
    lines.push('\\sectiontitle{Couverture des données}');
    lines.push(`\\bodytext{${escapeLatex(report.data_coverage)}}`);
  }

  if (report.descriptive_statistics.length > 0) {
    lines.push('\\sectiontitle{Statistiques descriptives}');
    lines.push(buildStatisticsTable(report));
  }

  if (report.regressions.length > 0) {
    lines.push('\\sectiontitle{Régressions linéaires}');
    lines.push(buildRegressionsTable(report));
  }

  if (report.charts.length > 0) {
    lines.push('\\sectiontitle{Visualisations préparées}');
    report.charts.forEach((chart) => {
      const chartLine = `${chart.title} (${chart.chart_type}) - ${chart.description || 'Visualisation prête pour le frontend.'}`;
      lines.push(`\\bulletitem{${escapeLatex(chartLine)}}`);
    });
  }

  if (report.used_resources.length > 0) {
    lines.push('\\sectiontitle{Ressources exploitées}');
    report.used_resources.forEach((resource) => {
      lines.push(
        `\\resourcecard{${escapeLatex(resource.dataset_title)}}{${escapeLatex(resource.resource_title)}}{${escapeLatex(resource.format.toUpperCase())}}{${escapeUrl(resource.resource_url)}}`,
      );
    });
  }

  if (report.selected_sources.length > 0) {
    lines.push('\\sectiontitle{Sources officielles}');
    report.selected_sources.forEach((source) => {
      lines.push(
        `\\sourcecard{${escapeLatex(source.title)}}{${escapeUrl(source.url)}}{${escapeLatex(source.description)}}{${escapeLatex(source.reason_for_selection)}}{${Math.round(source.confidence_score * 100)}}`,
      );
    });
  }

  if (report.limitations.length > 0) {
    lines.push('\\sectiontitle{Limites identifiées}');
    report.limitations.forEach((limitation) => {
      lines.push(`\\bulletitem{${escapeLatex(limitation)}}`);
    });
  }

  if (report.trace.length > 0) {
    lines.push('\\sectiontitle{Trace de génération}');
    report.trace.forEach((step, index) => {
      lines.push(`\\traceitem{${index + 1}}{${escapeLatex(step)}}`);
    });
  }

  return lines.join('\n');
}

function buildLatexDocument(fragment: string): string {
  const generatedAt = escapeLatex(
    new Intl.DateTimeFormat('fr-FR', {
      dateStyle: 'full',
      timeStyle: 'short',
    }).format(new Date()),
  );

  return `
\\documentclass[11pt,a4paper]{article}
\\usepackage[utf8]{inputenc}
\\usepackage[T1]{fontenc}
\\usepackage[french]{babel}
\\usepackage{graphicx}
\\usepackage[table]{xcolor}
\\usepackage{geometry}
\\usepackage{hyperref}
\\usepackage{tikz}
\\usepackage[most]{tcolorbox}
\\usepackage{tabularx}
\\usepackage{booktabs}
\\usepackage{microtype}
\\usepackage{pagecolor}
\\usepackage{fancyhdr}
\\geometry{margin=1.6cm}
\\setlength{\\parindent}{0pt}
\\setlength{\\parskip}{4pt}
\\definecolor{brandblue}{HTML}{002654}
\\definecolor{brandred}{HTML}{ED2939}
\\definecolor{brandsoft}{HTML}{F5F7FB}
\\definecolor{brandpaper}{HTML}{F8FAFC}
\\definecolor{brandink}{HTML}{0F172A}
\\pagecolor{brandpaper}
\\color{brandink}
\\hypersetup{
  colorlinks=true,
  urlcolor=brandred,
  linkcolor=brandblue
}
\\urlstyle{same}
\\emergencystretch=2em
\\sloppy
\\pagestyle{fancy}
\\fancyhf{}
\\fancyhead[L]{\\textcolor{brandblue}{\\textbf{MarianneAI}}}
\\fancyhead[R]{\\textcolor{brandred}{Rapport d'analyse}}
\\fancyfoot[C]{\\textcolor{brandblue}{\\thepage}}
\\renewcommand{\\headrulewidth}{0pt}
\\renewcommand{\\footrulewidth}{0pt}
\\newcommand{\\reportsubtitle}[1]{
  \\begin{tcolorbox}[colback=white,colframe=brandblue!18,arc=5pt,boxrule=0pt,left=10pt,right=10pt,top=10pt,bottom=10pt]
  #1
  \\end{tcolorbox}
}
\\newcommand{\\sectiontitle}[1]{
  \\vspace{8pt}
  {\\Large\\bfseries\\textcolor{brandblue}{#1}}\\par
  {\\color{brandred}\\rule{0.18\\linewidth}{1.2pt}}\\par\\vspace{4pt}
}
\\newcommand{\\bodytext}[1]{
  #1\\par
}
\\newcommand{\\highlightbox}[2]{
  \\begin{tcolorbox}[colback=white,colframe=brandblue,arc=5pt,boxrule=0.9pt,title={\\textbf{#1}},colbacktitle=brandblue,coltitle=white,left=10pt,right=10pt,top=10pt,bottom=10pt]
  #2
  \\end{tcolorbox}
}
\\newcommand{\\metriccard}[2]{
  \\begin{minipage}[t]{0.31\\linewidth}
    \\begin{tcolorbox}[colback=white,colframe=brandblue!12,arc=5pt,boxrule=0.4pt,left=8pt,right=8pt,top=8pt,bottom=8pt]
      {\\scriptsize\\textcolor{brandred}{\\textbf{#1}}}\\par
      \\vspace{3pt}
      {\\Large\\bfseries\\textcolor{brandblue}{#2}}
    \\end{tcolorbox}
  \\end{minipage}
}
\\newcommand{\\sourcecard}[5]{
  \\begin{tcolorbox}[colback=white,colframe=brandred!70!brandblue,arc=5pt,boxrule=0.7pt,left=10pt,right=10pt,top=10pt,bottom=10pt]
    {\\large\\bfseries\\textcolor{brandblue}{#1}}\\hfill {\\bfseries\\textcolor{brandred}{#5\\%}}\\par
    {\\small\\url{#2}}\\par\\vspace{4pt}
    #3\\par\\vspace{4pt}
    {\\bfseries Pourquoi cette source :} #4
  \\end{tcolorbox}
}
\\newcommand{\\resourcecard}[4]{
  \\begin{tcolorbox}[colback=white,colframe=brandblue!18,arc=5pt,boxrule=0.5pt,left=10pt,right=10pt,top=10pt,bottom=10pt]
    {\\bfseries\\textcolor{brandblue}{#1}}\\hfill {\\bfseries\\textcolor{brandred}{#3}}\\par
    {\\small #2}\\par\\vspace{4pt}
    {\\small\\url{#4}}
  \\end{tcolorbox}
}
\\newcommand{\\bulletitem}[1]{
  \\noindent\\textcolor{brandred}{\\large$\\bullet$}\\hspace{0.6em}
  \\begin{minipage}[t]{0.92\\linewidth}\\raggedright #1\\end{minipage}\\par\\vspace{6pt}
}
\\newcommand{\\traceitem}[2]{
  \\noindent\\textcolor{brandblue}{\\textbf{#1.}}\\hspace{0.6em}
  \\begin{minipage}[t]{0.9\\linewidth}\\raggedright #2\\end{minipage}\\par\\vspace{4pt}
}
\\begin{document}
\\begin{tcolorbox}[enhanced,colback=white,colframe=brandblue!12,arc=6pt,boxrule=0pt,left=12pt,right=12pt,top=12pt,bottom=12pt]
  \\begin{minipage}[c]{0.14\\linewidth}
    \\includegraphics[width=\\linewidth]{logo-mariene.png}
  \\end{minipage}
  \\hfill
  \\begin{minipage}[c]{0.8\\linewidth}
    {\\Huge\\bfseries\\textcolor{brandblue}{Rapport MarianneAI}}\\par
    {\\large\\textcolor{brandred}{Analyse sourcée et prête pour la démo}}\\par
    {\\small Généré le ${generatedAt}}
  \\end{minipage}
\\end{tcolorbox}

\\vspace{12pt}

${fragment}

\\end{document}
`.trim();
}

function escapeLatex(value: string): string {
  return normalizeText(value)
    .replace(/\\/g, '\\textbackslash{}')
    .replace(/([{}#$%&_])/g, '\\$1')
    .replace(/\^/g, '\\textasciicircum{}')
    .replace(/~/g, '\\textasciitilde{}')
    .replace(/\n/g, '\\\\');
}

function normalizeText(value: string): string {
  return value
    .replace(/\r\n/g, '\n')
    .replace(/[’]/g, "'")
    .replace(/[“”]/g, '"')
    .replace(/[–—]/g, '-')
    .replace(/…/g, '...')
    .replace(/\u00A0/g, ' ')
    .replace(/[•]/g, '-')
    .replace(/[^\u0009\u000A\u000D\u0020-\u00FF]/g, '');
}

function escapeUrl(value: string): string {
  return normalizeText(value)
    .replace(/\\/g, '/')
    .replace(/%/g, '\\%')
    .replace(/#/g, '\\#')
    .replace(/{/g, '\\{')
    .replace(/}/g, '\\}');
}

function formatMetric(value: number | null | undefined): string {
  if (value === null || value === undefined) {
    return 'N/A';
  }
  return new Intl.NumberFormat('fr-FR').format(value);
}

function formatDecimal(value: number | null | undefined): string {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return 'N/A';
  }
  return new Intl.NumberFormat('fr-FR', {
    minimumFractionDigits: 0,
    maximumFractionDigits: 2,
  }).format(value);
}

function buildStatisticsTable(report: QueryResponse): string {
  const rows = report.descriptive_statistics
    .slice(0, 6)
    .map((item) => {
      return `${escapeLatex(item.column)} & ${escapeLatex(formatDecimal(item.mean))} & ${escapeLatex(formatDecimal(item.median))} & ${escapeLatex(formatDecimal(item.min))} & ${escapeLatex(formatDecimal(item.max))} \\\\`;
    })
    .join('\n');

  return [
    '\\begin{tcolorbox}[colback=white,colframe=brandblue!12,arc=5pt,boxrule=0.4pt]',
    '\\small',
    '\\begin{tabularx}{\\linewidth}{>{\\raggedright\\arraybackslash}X>{\\raggedleft\\arraybackslash}p{0.15\\linewidth}>{\\raggedleft\\arraybackslash}p{0.15\\linewidth}>{\\raggedleft\\arraybackslash}p{0.15\\linewidth}>{\\raggedleft\\arraybackslash}p{0.15\\linewidth}}',
    '\\toprule',
    '\\textbf{Colonne} & \\textbf{Moy.} & \\textbf{Méd.} & \\textbf{Min} & \\textbf{Max}\\\\',
    '\\midrule',
    rows,
    '\\bottomrule',
    '\\end{tabularx}',
    '\\end{tcolorbox}',
  ].join('\n');
}

function buildRegressionsTable(report: QueryResponse): string {
  const rows = report.regressions
    .slice(0, 4)
    .map((item) => {
      return `${escapeLatex(item.feature_y)} / ${escapeLatex(item.feature_x)} & ${escapeLatex(formatDecimal(item.slope))} & ${escapeLatex(formatDecimal(item.intercept))} & ${escapeLatex(formatDecimal(item.r_squared))} \\\\`;
    })
    .join('\n');

  return [
    '\\begin{tcolorbox}[colback=white,colframe=brandred!18,arc=5pt,boxrule=0.4pt]',
    '\\small',
    '\\begin{tabularx}{\\linewidth}{>{\\raggedright\\arraybackslash}X>{\\raggedleft\\arraybackslash}p{0.16\\linewidth}>{\\raggedleft\\arraybackslash}p{0.16\\linewidth}>{\\raggedleft\\arraybackslash}p{0.16\\linewidth}}',
    '\\toprule',
    '\\textbf{Variables} & \\textbf{Pente} & \\textbf{Intercept} & \\textbf{R²}\\\\',
    '\\midrule',
    rows,
    '\\bottomrule',
    '\\end{tabularx}',
    '\\end{tcolorbox}',
  ].join('\n');
}

function buildPdfFileName(): string {
  const fileDate = new Intl.DateTimeFormat('fr-CA', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(new Date()).replaceAll('/', '-');

  return `rapport-marianneai-style-${fileDate}.pdf`;
}

async function runPdflatex(cwd: string, texFileName: string): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = spawn('pdflatex', ['-interaction=nonstopmode', '-halt-on-error', texFileName], {
      cwd,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let output = '';

    child.stdout.on('data', (chunk) => {
      output += chunk.toString();
    });

    child.stderr.on('data', (chunk) => {
      output += chunk.toString();
    });

    child.on('error', (error) => {
      reject(new Error(`pdflatex is unavailable: ${error.message}`));
    });

    child.on('close', (code) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(new Error(`pdflatex failed with code ${code}\n${output}`));
    });
  });
}
