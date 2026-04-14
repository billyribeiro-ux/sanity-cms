import {defineField, defineType} from 'sanity'

/**
 * Navigation menu — header/footer menus composed of links and sub-menus.
 */
export const navigation = defineType({
  name: 'navigation',
  title: 'Navigation',
  type: 'document',
  fields: [
    defineField({
      name: 'title',
      title: 'Title',
      type: 'string',
      description: 'Internal name (e.g. "Header", "Footer").',
      validation: (Rule) => Rule.required(),
    }),
    defineField({
      name: 'items',
      title: 'Items',
      type: 'array',
      of: [
        {
          type: 'object',
          name: 'navItem',
          fields: [
            {name: 'label', type: 'string', title: 'Label', validation: (Rule) => Rule.required()},
            {name: 'link', type: 'link', title: 'Link'},
            {
              name: 'children',
              type: 'array',
              title: 'Sub-items',
              of: [
                {
                  type: 'object',
                  fields: [
                    {name: 'label', type: 'string', title: 'Label'},
                    {name: 'link', type: 'link', title: 'Link'},
                  ],
                },
              ],
            },
          ],
          preview: {select: {title: 'label'}},
        },
      ],
    }),
  ],
})
