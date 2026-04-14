import {defineField, defineType} from 'sanity'

/**
 * Accessible image — requires alt text. Use instead of the bare `image` type.
 */
export const imageWithAlt = defineType({
  name: 'imageWithAlt',
  title: 'Image',
  type: 'image',
  options: {hotspot: true},
  fields: [
    defineField({
      name: 'alt',
      title: 'Alt text',
      type: 'string',
      description: 'Describe the image for screen readers and when the image fails to load.',
      validation: (Rule) =>
        Rule.custom((alt, context) => {
          // Only required if an image asset is set
          const parent = context.parent as {asset?: unknown} | undefined
          if (parent?.asset && !alt) return 'Alt text is required when an image is set.'
          return true
        }),
    }),
    defineField({
      name: 'caption',
      title: 'Caption',
      type: 'string',
    }),
  ],
})
