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

    if (isGeminiConfigured(process.env['GEMINI_API_KEY'])) {
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
    'Le backend ajoute déjà le logo logo-mariene.png dans l’en-tête et compile le document.',
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
  const lines = [
    '\\reportsubtitle{Synthèse citoyenne mise en page automatiquement aux couleurs de MarianneAI.}',
    `\\highlightbox{Résumé exécutif}{${escapeLatex(report.answer)}}`,
    '\\sectiontitle{Question posée}',
    `\\bodytext{${escapeLatex(report.user_query)}}`,
    '\\sectiontitle{Réponse détaillée}',
    `\\bodytext{${escapeLatex(report.answer)}}`,
  ];

  if (report.selected_sources.length > 0) {
    lines.push('\\sectiontitle{Sources officielles}');
    report.selected_sources.forEach((source) => {
      lines.push(
        `\\sourcecard{${escapeLatex(source.title)}}{${escapeLatex(source.url)}}{${escapeLatex(source.description)}}{${escapeLatex(source.reason_for_selection)}}{${Math.round(source.confidence_score * 100)}}`,
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
\\geometry{margin=1.6cm}
\\setlength{\\parindent}{0pt}
\\setlength{\\parskip}{4pt}
\\definecolor{brandblue}{HTML}{002654}
\\definecolor{brandred}{HTML}{ED2939}
\\definecolor{brandsoft}{HTML}{F5F7FB}
\\hypersetup{
  colorlinks=true,
  urlcolor=brandred,
  linkcolor=brandblue
}
\\newcommand{\\reportsubtitle}[1]{
  \\begin{tcolorbox}[colback=brandsoft,colframe=brandblue!18,arc=3pt,boxrule=0pt]
  #1
  \\end{tcolorbox}
}
\\newcommand{\\sectiontitle}[1]{
  \\vspace{4pt}
  {\\Large\\bfseries\\textcolor{brandblue}{#1}}\\par\\vspace{4pt}
}
\\newcommand{\\bodytext}[1]{
  #1\\par
}
\\newcommand{\\highlightbox}[2]{
  \\begin{tcolorbox}[colback=brandblue!5,colframe=brandblue,arc=3pt,boxrule=0.8pt,title={\\textbf{#1}},colbacktitle=brandblue,coltitle=white]
  #2
  \\end{tcolorbox}
}
\\newcommand{\\sourcecard}[5]{
  \\begin{tcolorbox}[colback=white,colframe=brandred!70!brandblue,arc=3pt,boxrule=0.7pt]
    {\\large\\bfseries\\textcolor{brandblue}{#1}}\\hfill {\\bfseries\\textcolor{brandred}{#5\\%}}\\par
    {\\small\\texttt{#2}}\\par\\vspace{4pt}
    #3\\par\\vspace{4pt}
    {\\bfseries Pourquoi cette source :} #4
  \\end{tcolorbox}
}
\\newcommand{\\bulletitem}[1]{
  \\noindent\\textcolor{brandred}{\\large$\\bullet$}\\hspace{0.6em}
  \\begin{minipage}[t]{0.92\\linewidth}#1\\end{minipage}\\par\\vspace{4pt}
}
\\newcommand{\\traceitem}[2]{
  \\noindent\\textcolor{brandblue}{\\textbf{#1.}}\\hspace{0.6em}
  \\begin{minipage}[t]{0.9\\linewidth}#2\\end{minipage}\\par\\vspace{4pt}
}
\\begin{document}
\\begin{minipage}[c]{0.16\\linewidth}
  \\includegraphics[width=\\linewidth]{logo-mariene.png}
\\end{minipage}
\\hfill
\\begin{minipage}[c]{0.8\\linewidth}
  {\\Huge\\bfseries\\textcolor{brandblue}{Rapport MarianneAI}}\\par
  {\\large\\textcolor{brandred}{PDF stylé généré côté backend}}\\par
  {\\small Généré le ${generatedAt}}
\\end{minipage}

\\vspace{10pt}
{\\color{brandblue}\\rule{\\linewidth}{1.4pt}}
\\vspace{10pt}

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
  const normalized = value
    .replace(/\r\n/g, '\n')
    .replace(/[’]/g, "'")
    .replace(/[“”]/g, '"')
    .replace(/[–—]/g, '-')
    .replace(/…/g, '...')
    .replace(/\u00A0/g, ' ')
    .replace(/[•]/g, '-');

  return Array.from(normalized)
    .filter((char) => {
      const codePoint = char.codePointAt(0) ?? 0;

      return codePoint === 0x09 || codePoint === 0x0a || codePoint === 0x0d || (codePoint >= 0x20 && codePoint <= 0xff);
    })
    .join('');
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
    const process = spawn('pdflatex', ['-interaction=nonstopmode', '-halt-on-error', texFileName], {
      cwd,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let output = '';

    process.stdout.on('data', (chunk) => {
      output += chunk.toString();
    });

    process.stderr.on('data', (chunk) => {
      output += chunk.toString();
    });

    process.on('close', (code) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(new Error(`pdflatex failed with code ${code}\n${output}`));
    });
  });
}
