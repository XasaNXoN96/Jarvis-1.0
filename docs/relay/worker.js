// Jarvis relay on Cloudflare Workers: a small server OUTSIDE the blocked network that forwards requests of Jarvis to the allowed hosts only.
// Deploy it in YOUR OWN Cloudflare account (see README.md); nothing here is shared with anybody.
//
// Address of a request:   https://<your-worker>.workers.dev/<SECRET>/<host>/<path...>
// Example (Telegram):     https://<your-worker>.workers.dev/<SECRET>/api.telegram.org/bot<TOKEN>/getMe
//
// - SECRET is a long random word kept in the variable RELAY_SECRET of the worker; a wrong word gets 404 and no reason.
// - Only the hosts of ALLOWED are reachable: the relay is not an open proxy.
// - The key of the AI / the token of the bot travel in the request as usual; the relay does not log or keep anything.
const ALLOWED = new Set([
  'api.openai.com',
  'api.anthropic.com',
  'generativelanguage.googleapis.com',
  'api.telegram.org',
  'api.open-meteo.com',
  'huggingface.co',
  'cdn-lfs.huggingface.co',
  'github.com',
  'api.github.com',
  'objects.githubusercontent.com',
]);

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const parts = url.pathname.split('/').filter(Boolean);
    // /<secret>/<host>/<path...>
    if (!env.RELAY_SECRET || env.RELAY_SECRET.length < 16 || parts.length < 2 || parts[0] !== env.RELAY_SECRET) {
      return new Response('Not found', { status: 404 });
    }
    const host = parts[1].toLowerCase();
    if (!ALLOWED.has(host)) {
      return new Response('Not found', { status: 404 });
    }
    const target = 'https://' + host + '/' + parts.slice(2).join('/') + url.search;
    const headers = new Headers(request.headers);
    for (const h of ['host', 'cf-connecting-ip', 'cf-ipcountry', 'cf-ray', 'cf-visitor', 'x-forwarded-for', 'x-real-ip']) headers.delete(h);
    const init = { method: request.method, headers, redirect: 'manual' };
    if (request.method !== 'GET' && request.method !== 'HEAD') init.body = request.body;
    return fetch(target, init);
  },
};
