import * as objects from './objects'
import * as documents from './documents'

export * from './objects'
export * from './documents'

/**
 * All schema types bundled as one array — drop straight into `schema.types` in sanity.config.ts.
 *
 * @example
 *   import {allSchemaTypes} from '@cms-kit/schemas'
 *   defineConfig({schema: {types: allSchemaTypes}})
 */
export const allSchemaTypes = [
  ...Object.values(objects),
  ...Object.values(documents),
]
