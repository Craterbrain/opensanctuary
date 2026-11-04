import { describe, test, expect } from 'bun:test';
import { projectTextFromElements } from '../src/editor/slide_editor';
import { SlideElement, defaultTransform } from '../src/editor/types';

describe('Slide text projection', () => {
  test('projects text from single and multiple TextBlocks cleanly', () => {
    const elements: SlideElement[] = [
      {
        type: 'TextBlock',
        id: 'tb-1',
        transform: defaultTransform(),
        block: {
          runs: [{ text: 'Praise God from whom all blessings flow' }]
        }
      },
      {
        type: 'TextBlock',
        id: 'tb-2',
        transform: defaultTransform(),
        block: {
          runs: [{ text: 'Praise Him all creatures here below' }]
        }
      }
    ];

    const projected = projectTextFromElements(elements);
    expect(projected).toBe('Praise God from whom all blessings flow\n\nPraise Him all creatures here below');
  });

  test('projects text from Tables formatting rows with pipes', () => {
    const elements: SlideElement[] = [
      {
        type: 'Table',
        id: 'tbl-1',
        transform: defaultTransform(),
        rows: 2,
        cols: 2,
        cells: [
          ['Time', 'Event'],
          ['10:00 AM', 'Sunday Worship']
        ]
      }
    ];

    const projected = projectTextFromElements(elements);
    expect(projected).toBe('Time | Event\n10:00 AM | Sunday Worship');
  });

  test('projects text recursively from Group elements without data loss', () => {
    const elements: SlideElement[] = [
      {
        type: 'Group',
        id: 'grp-1',
        transform: defaultTransform(),
        children: [
          {
            type: 'TextBlock',
            id: 'child-1',
            transform: defaultTransform(),
            block: { runs: [{ text: 'Group Title' }] }
          },
          {
            type: 'TextBlock',
            id: 'child-2',
            transform: defaultTransform(),
            block: { runs: [{ text: 'Group Description' }] }
          }
        ]
      }
    ];

    const projected = projectTextFromElements(elements);
    expect(projected).toBe('Group Title\n\nGroup Description');
  });

  test('ignores shapes, lines, images, and videos in text projection', () => {
    const elements: SlideElement[] = [
      {
        type: 'Shape',
        id: 'shp-1',
        transform: defaultTransform(),
        shape_kind: 'rectangle',
        fill_color: '#333333'
      },
      {
        type: 'Line',
        id: 'ln-1',
        transform: defaultTransform(),
        line_kind: 'straight',
        color: '#ffffff'
      },
      {
        type: 'Image',
        id: 'img-1',
        transform: defaultTransform(),
        file_path: '/photos/cross.jpg'
      },
      {
        type: 'TextBlock',
        id: 'tb-1',
        transform: defaultTransform(),
        block: { runs: [{ text: 'Pure text content' }] }
      }
    ];

    const projected = projectTextFromElements(elements);
    expect(projected).toBe('Pure text content');
  });
});
