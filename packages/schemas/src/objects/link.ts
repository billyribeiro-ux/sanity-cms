import {defineField, defineType} from 'sanity'

/**
 * Polymorphic link: either an internal reference to a document (resolved at render time)
 * or an external URL.
 */
export const link = defineType({
  name: 'link',
  title: 'Link',
  type: 'object',
  fields: [
    defineField({
      name: 'type',
      title: 'Type',
      type: 'string',
      options: {
        list: [
          {title: 'Internal', value: 'internal'},
          {title: 'External', value: 'external'},
        ],
        layout: 'radio',
      },
      initialValue: 'internal',
      validation: (Rule) => Rule.required(),
    }),
    defineField({
      name: 'internal',
      title: 'Internal page',
      type: 'reference',
      to: [{type: 'page'}],
      hidden: ({parent}) => parent?.type !== 'internal',
      validation: (Rule) =>
        Rule.custom((val, ctx) => {
          const parent = ctx.parent as {type?: string} | undefined
          if (parent?.type === 'internal' && !val) return 'Pick a page'
          return true
        }),
    }),
    defineField({
      name: 'url',
      title: 'External URL',
      type: 'url',
      hidden: ({parent}) => parent?.type !== 'external',
      validation: (Rule) =>
        Rule.uri({allowRelative: false, scheme: ['http', 'https', 'mailto', 'tel']}).custom((val, ctx) => {
          const parent = ctx.parent as {type?: string} | undefined
          if (parent?.type === 'external' && !val) return 'URL required'
          return true
        }),
    }),
    defineField({
      name: 'openInNewTab',
      title: 'Open in new tab',
      type: 'boolean',
      initialValue: false,
    }),
  ],
})
