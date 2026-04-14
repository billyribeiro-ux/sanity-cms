import {defineKitConfig} from '@cms-kit/studio-config'

/**
 * Self-hosted by default — all data-layer calls go to content-lake-rs
 * running at http://localhost:3030. Nothing talks to api.sanity.io.
 *
 * To use Sanity's hosted backend instead, unset SANITY_STUDIO_API_HOST
 * and replace `projectId` with your real Sanity project ID.
 */
export default defineKitConfig({
  name: 'default',
  title: 'Studio',
  projectId: process.env.SANITY_STUDIO_PROJECT_ID || 'default',
  dataset: process.env.SANITY_STUDIO_DATASET || 'production',
  apiHost: process.env.SANITY_STUDIO_API_HOST || 'http://localhost:3030',
  singletons: ['siteSettings'],
})
