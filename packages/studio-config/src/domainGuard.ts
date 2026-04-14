/**
 * Global fetch interceptor that blocks any outbound request to Sanity-owned
 * domains. Installed by {@link defineKitConfig} whenever `apiHost` is set to
 * a self-hosted URL.
 *
 * Why it's needed: even with `apiHost` overridden, the Studio and @sanity/client
 * still hard-code a handful of hosts for telemetry, Sentry, the Media Library,
 * CDN, and Canvas. Those make this project *not* fully self-hosted unless
 * blocked at the network layer.
 *
 * What it blocks:
 *   - api.sanity.io, api.sanity.work          (telemetry / Sentry / Canvas)
 *   - apicdn.sanity.io, sanity-cdn.com, sanity-cdn.work  (query CDN)
 *   - media.sanity.io, media.sanity.work      (Media Library frontend)
 *   - sentry.sanity.io                        (error reporting tunnel)
 *   - *.sanity.studio                         (Studio hosting)
 *
 * What it leaves alone:
 *   - Plain `<a href="https://www.sanity.io/docs/…">` documentation links —
 *     these are navigations, not fetches. They only "leak" if a user clicks.
 *
 * Runs once per page load; subsequent calls are no-ops.
 */

const SANITY_HOSTS = [
  /(^|\.)sanity\.io$/,
  /(^|\.)sanity\.work$/,
  /(^|\.)sanity-cdn\.com$/,
  /(^|\.)sanity-cdn\.work$/,
  /(^|\.)sanity\.studio$/,
  /(^|\.)sanity-cdn\.io$/,
]

let installed = false

export interface DomainGuardOptions {
  /** Log every blocked request to console.warn. Default true. */
  log?: boolean
  /** Extra hostnames to allow through (useful if you explicitly want media.sanity.io). */
  allow?: string[]
}

/**
 * Monkey-patch `window.fetch` (and the `XMLHttpRequest` opener) to reject any
 * request whose URL points at a Sanity-owned host. No-op in non-browser
 * environments.
 */
export function installSanityDomainGuard(options: DomainGuardOptions = {}): void {
  if (installed) return
  if (typeof globalThis === 'undefined') return
  const g = globalThis as unknown as {fetch?: typeof fetch; XMLHttpRequest?: typeof XMLHttpRequest}
  if (!g.fetch) return

  const log = options.log ?? true
  const allow = new Set(options.allow ?? [])

  const isBlocked = (url: string): boolean => {
    let parsed: URL
    try {
      parsed = new URL(url, typeof location !== 'undefined' ? location.href : 'http://localhost/')
    } catch {
      return false
    }
    const host = parsed.hostname
    if (allow.has(host)) return false
    return SANITY_HOSTS.some((re) => re.test(host))
  }

  const blockedResponse = () =>
    new Response('{"error":"blocked by cms-kit domain guard"}', {
      status: 451,
      statusText: 'Blocked',
      headers: {'content-type': 'application/json'},
    })

  const originalFetch = g.fetch.bind(g) as typeof fetch
  g.fetch = (input, init) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : (input as Request).url
    if (isBlocked(url)) {
      if (log) console.warn(`[cms-kit] blocked outbound fetch to ${url}`)
      return Promise.resolve(blockedResponse())
    }
    return originalFetch(input, init)
  }

  if (g.XMLHttpRequest) {
    const XHR = g.XMLHttpRequest
    const origOpen = XHR.prototype.open
    XHR.prototype.open = function (
      method: string,
      url: string | URL,
      async?: boolean,
      username?: string | null,
      password?: string | null,
    ): void {
      const urlStr = typeof url === 'string' ? url : url.href
      if (isBlocked(urlStr)) {
        if (log) console.warn(`[cms-kit] blocked outbound XHR to ${urlStr}`)
        // Re-route blocked XHRs to about:blank so the request never leaves.
        return origOpen.call(this, method, 'about:blank', async ?? true, username ?? null, password ?? null)
      }
      return origOpen.call(this, method, url, async ?? true, username ?? null, password ?? null)
    }
  }

  // Mark as installed AFTER patching so a second call from defineKitConfig
  // doesn't wrap twice.
  installed = true
}

/** Test-only reset. Do not call in production. */
export function __resetDomainGuardForTests(): void {
  installed = false
}
