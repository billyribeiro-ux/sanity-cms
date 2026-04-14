import {defineConfig, type Config, type PluginOptions, type SchemaTypeDefinition} from 'sanity'
import {structureTool} from 'sanity/structure'
import {allSchemaTypes} from '@cms-kit/schemas'

export interface KitConfigOptions {
  projectId: string
  dataset: string
  title?: string
  name?: string
  /** Additional schema types. Appended to the cms-kit defaults. */
  schemaTypes?: SchemaTypeDefinition[]
  /** Plugins to load in addition to the defaults (structureTool is always included). */
  plugins?: PluginOptions[]
  /** Set to true to replace (not append) the default cms-kit schema types. */
  replaceSchemaTypes?: boolean
  /** Singleton documents that should be pinned to one instance (e.g. `siteSettings`). */
  singletons?: string[]
  /**
   * Base URL of the self-hosted Content Lake API (content-lake-rs).
   * When set, the Studio sends all data-layer requests here instead of
   * `api.sanity.io`. Example: `http://localhost:3030`.
   *
   * Leave undefined to use Sanity's hosted backend.
   */
  apiHost?: string
  /**
   * Whether to route requests under `{apiHost}/v1/...` instead of
   * `{projectId}.{apiHost}/v1/...`. Required when self-hosting, since
   * we don't wildcard-subdomain the backend. Defaults to `true` when
   * `apiHost` is set.
   */
  useProjectHostname?: boolean
}

/**
 * Opinionated wrapper around `defineConfig` with cms-kit schema types preloaded
 * and optional singleton handling.
 *
 * @example
 *   import {defineKitConfig} from '@cms-kit/studio-config'
 *
 *   export default defineKitConfig({
 *     projectId: 'abc123',
 *     dataset: 'production',
 *     singletons: ['siteSettings'],
 *   })
 */
export function defineKitConfig(options: KitConfigOptions): Config {
  const {
    projectId,
    dataset,
    title = 'Studio',
    name = 'default',
    schemaTypes = [],
    plugins = [],
    replaceSchemaTypes = false,
    singletons = [],
    apiHost,
    useProjectHostname,
  } = options

  const types = replaceSchemaTypes
    ? schemaTypes
    : [...(allSchemaTypes as SchemaTypeDefinition[]), ...schemaTypes]

  // When self-hosting, default useProjectHostname to false so client URLs
  // are path-based (`/v1/...`) instead of subdomain-based.
  const resolvedUseProjectHostname =
    useProjectHostname ?? (apiHost ? false : undefined)

  return defineConfig({
    name,
    title,
    projectId,
    dataset,
    ...(apiHost ? {apiHost} : {}),
    ...(resolvedUseProjectHostname !== undefined
      ? {useProjectHostname: resolvedUseProjectHostname}
      : {}),
    schema: {
      types,
      // Hide "create new" buttons and list items for singletons.
      templates: (prev) => prev.filter((t) => !singletons.includes(t.id)),
    },
    document: {
      // Hide singleton-type docs from the "new document" action menus.
      newDocumentOptions: (prev, {creationContext}) =>
        creationContext.type === 'global' ? prev.filter((t) => !singletons.includes(t.templateId)) : prev,
      actions: (prev, {schemaType}) =>
        singletons.includes(schemaType)
          ? prev.filter(({action}) => !['unpublish', 'delete', 'duplicate'].includes(action as string))
          : prev,
    },
    plugins: [
      structureTool({
        structure: (S) => {
          const singletonItems = singletons.map((id) =>
            S.listItem().title(titleFor(id)).id(id).child(S.document().schemaType(id).documentId(id)),
          )
          const defaultItems = S.documentTypeListItems().filter(
            (item) => !singletons.includes(item.getId() as string),
          )
          return S.list()
            .title('Content')
            .items([...singletonItems, ...(singletonItems.length ? [S.divider()] : []), ...defaultItems])
        },
      }),
      ...plugins,
    ],
  })
}

function titleFor(id: string): string {
  return id.replace(/([A-Z])/g, ' $1').replace(/^./, (c) => c.toUpperCase())
}
