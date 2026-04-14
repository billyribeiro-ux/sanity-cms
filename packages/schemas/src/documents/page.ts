import {defineField, defineType} from 'sanity'

/**
 * Generic page document — title, slug, hero, body, SEO.
 */
export const page = defineType({
  name: 'page',
  title: 'Page',
  type: 'document',
  groups: [
    {name: 'content', title: 'Content', default: true},
    {name: 'seo', title: 'SEO'},
  ],
  fields: [
    defineField({
      name: 'title',
      title: 'Title',
      type: 'string',
      group: 'content',
      validation: (Rule) => Rule.required().max(120),
    }),
    defineField({
      name: 'slug',
      title: 'Slug',
      type: 'slug',
      group: 'content',
      options: {source: 'title', maxLength: 96},
      validation: (Rule) => Rule.required(),
    }),
    defineField({
      name: 'hero',
      title: 'Hero image',
      type: 'imageWithAlt',
      group: 'content',
    }),
    defineField({
      name: 'body',
      title: 'Body',
      type: 'richText',
      group: 'content',
    }),
    defineField({
      name: 'seo',
      title: 'SEO',
      type: 'seo',
      group: 'seo',
    }),
  ],
  preview: {
    select: {title: 'title', slug: 'slug.current', media: 'hero'},
    prepare({title, slug, media}) {
      return {title: title || 'Untitled page', subtitle: slug ? `/${slug}` : '', media}
    },
  },
})
