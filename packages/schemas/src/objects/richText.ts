import {defineArrayMember, defineField, defineType} from 'sanity'

/**
 * Opinionated portable-text rich text block.
 * Includes headings, lists, links, and inline images.
 */
export const richText = defineType({
  name: 'richText',
  title: 'Rich text',
  type: 'array',
  of: [
    defineArrayMember({
      type: 'block',
      styles: [
        {title: 'Normal', value: 'normal'},
        {title: 'H2', value: 'h2'},
        {title: 'H3', value: 'h3'},
        {title: 'H4', value: 'h4'},
        {title: 'Quote', value: 'blockquote'},
      ],
      lists: [
        {title: 'Bullet', value: 'bullet'},
        {title: 'Numbered', value: 'number'},
      ],
      marks: {
        decorators: [
          {title: 'Bold', value: 'strong'},
          {title: 'Italic', value: 'em'},
          {title: 'Code', value: 'code'},
        ],
        annotations: [
          defineField({
            name: 'link',
            title: 'Link',
            type: 'object',
            fields: [
              {name: 'href', type: 'url', title: 'URL'},
              {name: 'openInNewTab', type: 'boolean', title: 'Open in new tab', initialValue: false},
            ],
          }),
        ],
      },
    }),
    defineArrayMember({type: 'imageWithAlt'}),
  ],
})
