import {
  AngularNodeAppEngine,
  createNodeRequestHandler,
  isMainModule,
  writeResponseToNodeResponse,
} from '@angular/ssr/node';
import express from 'express';
import {join} from 'node:path';

const browserDistFolder = join(import.meta.dirname, '../browser');

const app = express();
const angularApp = new AngularNodeAppEngine();

app.use('/api', express.json());

app.all('/api/{*proxyPath}', async (req, res) => {
  const backendUrl = process.env['BACKEND_URL'];

  if (!backendUrl) {
    res.status(503).json({error: 'BACKEND_URL is not configured'});
    return;
  }

  try {
    const proxyPathParam = req.params['proxyPath'];
    const proxyPath = Array.isArray(proxyPathParam)
      ? proxyPathParam.join('/')
      : (proxyPathParam ?? '');
    const queryStringIndex = req.originalUrl.indexOf('?');
    const queryString = queryStringIndex >= 0 ? req.originalUrl.slice(queryStringIndex) : '';
    const targetUrl = new URL(
      `${proxyPath}${queryString}`,
      backendUrl.endsWith('/') ? backendUrl : `${backendUrl}/`,
    );
    const headers = new Headers();
    const acceptHeader = req.header('accept');
    const contentTypeHeader = req.header('content-type');

    if (acceptHeader) {
      headers.set('accept', acceptHeader);
    }

    if (contentTypeHeader) {
      headers.set('content-type', contentTypeHeader);
    }

    const response = await fetch(targetUrl, {
      method: req.method,
      headers,
      body:
        req.method === 'GET' || req.method === 'HEAD' || req.body === undefined
          ? undefined
          : JSON.stringify(req.body),
    });
    const responseContentType = response.headers.get('content-type');

    if (responseContentType) {
      res.setHeader('content-type', responseContentType);
    }

    res.status(response.status).send(await response.text());
  } catch (error) {
    console.error('Error while proxying backend request', error);
    res.status(502).json({error: 'Backend request failed'});
  }
});

app.use(
  express.static(browserDistFolder, {
    maxAge: '1y',
    index: false,
    redirect: false,
  }),
);

app.use((req, res, next) => {
  angularApp
    .handle(req)
    .then((response) =>
      response ? writeResponseToNodeResponse(response, res) : next(),
    )
    .catch(next);
});

if (isMainModule(import.meta.url) || process.env['pm_id']) {
  const port = process.env['PORT'] || 4000;
  app.listen(port, (error) => {
    if (error) {
      throw error;
    }

    console.log(`Node Express server listening on http://localhost:${port}`);
  });
}

export const reqHandler = createNodeRequestHandler(app);
