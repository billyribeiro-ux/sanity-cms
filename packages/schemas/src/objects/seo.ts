import {defineField, defineType} from 'sanity'

/**
 * SEO object — drop into any document as a field of type "seo".
 */
export const seo = defineType({
  name: 'seo',
  title: 'SEO',
  type: 'object',
  options: {collapsible: true, collapsed: true},
  fields: [
    defineField({
      name: 'title',
      title: 'Meta title',
      type: 'string',
      description: 'Overrides the document title in <title>. Keep under 60 chars.',
      validation: (Rule) => Rule.max(60).warning('Titles over 60 chars get truncated in search results.'),
    }),
    defineField({
      name: 'description',
      title: 'Meta description',
      type: 'text',
      rows: 3,
      description: 'Shown in search results. Aim for 140–160 chars.',
      validation: (Rule) => Rule.max(160).warning('Descriptions over 160 chars get truncated.'),
    }),
    defineField({
      name: 'ogImage',
      title: 'Social share image',
      type: 'image',
      description: '1200×630 recommended.',
      options: {hotspot: true},
    }),
    defineField({
      name: 'noIndex',
      title: 'Hide from search engines',
      type: 'boolean',
      initialValue: false,
    }),
    defineField({
      name: 'canonicalUrl',
      title: 'Canonical URL',
      type: 'url',
      description: 'Use when this content is published elsewhere as the source of truth.',
    }),
  ],
})
