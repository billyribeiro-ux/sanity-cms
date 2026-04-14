import {defineKitConfig} from '@cms-kit/studio-config'

/**
 * Replace projectId + dataset with your client's values, then run `pnpm dev`.
 */
export default defineKitConfig({
  name: 'default',
  title: 'Studio',
  projectId: process.env.SANITY_STUDIO_PROJECT_ID || 'REPLACE_ME',
  dataset: process.env.SANITY_STUDIO_DATASET || 'production',
  singletons: ['siteSettings'],
})
