import { ElementTransform, SlideElement } from './types';

export interface TableElementProps {
  rows: number;
  cols: number;
  cells: string[][];
  opacity?: number;
}

export function createDefaultTable(
  rows: number = 3,
  cols: number = 3,
  transform: ElementTransform
): SlideElement {
  const cells: string[][] = [];
  for (let r = 0; r < rows; r++) {
    const row: string[] = [];
    for (let c = 0; c < cols; c++) {
      row.push(r === 0 ? `Header ${c + 1}` : `Cell ${r},${c}`);
    }
    cells.push(row);
  }

  return {
    type: 'Table',
    id: `tbl-${Date.now()}-${Math.random().toString(36).substr(2, 4)}`,
    transform,
    rows,
    cols,
    cells
  };
}

export function renderTableDOM(
  containerEl: HTMLElement,
  props: TableElementProps,
  isEditing: boolean = false,
  onCellsChange?: (cells: string[][]) => void
): void {
  // See note in shape_library.ts: don't touch containerEl's width/height here,
  // the caller already sizes it absolutely in px.
  containerEl.innerHTML = '';
  containerEl.style.opacity = `${props.opacity ?? 1.0}`;
  containerEl.style.overflow = 'hidden';

  const table = document.createElement('table');
  table.style.width = '100%';
  table.style.height = '100%';
  table.style.borderCollapse = 'collapse';
  table.style.tableLayout = 'fixed';

  const currentCells = props.cells || [];

  for (let r = 0; r < props.rows; r++) {
    const tr = document.createElement('tr');
    for (let c = 0; c < props.cols; c++) {
      const td = document.createElement('td');
      td.style.border = '1px solid rgba(255, 255, 255, 0.4)';
      td.style.padding = '4px 8px';
      td.style.color = '#ffffff';
      td.style.fontSize = '14px';
      td.style.textAlign = 'center';
      td.style.verticalAlign = 'middle';

      const cellText = (currentCells[r] && currentCells[r][c]) ? currentCells[r][c] : '';
      td.textContent = cellText;

      if (isEditing) {
        td.contentEditable = 'true';
        td.style.outline = 'none';
        td.oninput = () => {
          if (!currentCells[r]) currentCells[r] = [];
          currentCells[r][c] = td.textContent || '';
          if (onCellsChange) onCellsChange(currentCells);
        };
      }

      tr.appendChild(td);
    }
    table.appendChild(tr);
  }

  containerEl.appendChild(table);
}
