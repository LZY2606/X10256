//! Grid rasterizer. Dark/clear polarity is applied strictly in operation
//! order. The grid is a derived structure: cell size is a rule parameter and
//! the source geometry (with raw provenance) is retained separately.

pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub cell_nm: i64,
    pub min_x: i64,
    pub min_y: i64,
    pub mask: Vec<bool>,
}

impl Raster {
    pub fn new(bbox: (i64, i64, i64, i64), cell_nm: i64) -> Self {
        let (min_x, min_y, max_x, max_y) = bbox;
        let width = (((max_x - min_x) / cell_nm) + 2) as u32;
        let height = (((max_y - min_y) / cell_nm) + 2) as u32;
        let mask = vec![false; (width as usize) * (height as usize)];
        Self { width, height, cell_nm, min_x, min_y, mask }
    }

    #[inline]
    fn idx(&self, cx: i64, cy: i64) -> Option<usize> {
        if cx < 0 || cy < 0 || cx >= self.width as i64 || cy >= self.height as i64 {
            return None;
        }
        Some(cy as usize * self.width as usize + cx as usize)
    }

    #[inline]
    fn cell_of(&self, x: i64, y: i64) -> (i64, i64) {
        (
            (x - self.min_x).div_euclid(self.cell_nm),
            (y - self.min_y).div_euclid(self.cell_nm),
        )
    }

    fn set(&mut self, x: i64, y: i64, value: bool) {
        let (cx, cy) = self.cell_of(x, y);
        if let Some(i) = self.idx(cx, cy) {
            self.mask[i] = value;
        }
    }

    pub fn fill_circle(&mut self, cx_nm: i64, cy_nm: i64, r_nm: i64, value: bool) {
        if r_nm < 0 {
            return;
        }
        let step = self.cell_nm.max(1);
        let r_cells = (r_nm / step) as i64;
        let (ccx, ccy) = self.cell_of(cx_nm, cy_nm);
        for gy in -r_cells..=r_cells {
            for gx in -r_cells..=r_cells {
                if gx * gx + gy * gy <= r_cells * r_cells + r_cells {
                    if let Some(i) = self.idx(ccx + gx, ccy + gy) {
                        self.mask[i] = value;
                    }
                }
            }
        }
    }

    pub fn fill_line(&mut self, ax: i64, ay: i64, bx: i64, by: i64, width_nm: i64, value: bool) {
        let dx = bx - ax;
        let dy = by - ay;
        let length = ((dx as f64).hypot(dy as f64)).round() as i64;
        let steps = (length / self.cell_nm).max(1);
        for s in 0..=steps {
            let t_num = s as i128;
            let x = ax + ((dx as i128 * t_num) / steps as i128) as i64;
            let y = ay + ((dy as i128 * t_num) / steps as i128) as i64;
            self.fill_circle(x, y, width_nm / 2, value);
        }
    }

    pub fn fill_polygon(&mut self, pts: &[(i64, i64)], value: bool) {
        if pts.len() < 3 {
            return;
        }
        let mut min_y = i64::MAX;
        let mut max_y = i64::MIN;
        for (_, y) in pts {
            min_y = min_y.min(*y);
            max_y = max_y.max(*y);
        }
        let start_cy = (min_y - self.min_y).div_euclid(self.cell_nm);
        let end_cy = (max_y - self.min_y).div_euclid(self.cell_nm);
        let mut xs: Vec<f64> = Vec::new();
        for cy in start_cy..=end_cy {
            let y_nm = self.min_y + cy * self.cell_nm;
            xs.clear();
            for e in pts.windows(2) {
                let (x1, y1) = (e[0].0 as f64, e[0].1 as f64);
                let (x2, y2) = (e[1].0 as f64, e[1].1 as f64);
                if (y1 <= y_nm as f64 && y2 > y_nm as f64) || (y2 <= y_nm as f64 && y1 > y_nm as f64) {
                    let t = (y_nm as f64 - y1) / (y2 - y1);
                    xs.push(x1 + t * (x2 - x1));
                }
            }
            if pts.first() != pts.last() {
                let (x1, y1) = (pts.last().unwrap().0 as f64, pts.last().unwrap().1 as f64);
                let (x2, y2) = (pts.first().unwrap().0 as f64, pts.first().unwrap().1 as f64);
                if (y1 <= y_nm as f64 && y2 > y_nm as f64) || (y2 <= y_nm as f64 && y1 > y_nm as f64) {
                    let t = (y_nm as f64 - y1) / (y2 - y1);
                    xs.push(x1 + t * (x2 - x1));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < xs.len() {
                let x0 = xs[i] as i64;
                let x1 = xs[i + 1] as i64;
                let mut cx = (x0 - self.min_x).div_euclid(self.cell_nm);
                let cxe = (x1 - self.min_x).div_euclid(self.cell_nm);
                while cx <= cxe {
                    if let Some(idx) = self.idx(cx, cy) {
                        self.mask[idx] = value;
                    }
                    cx += 1;
                }
                i += 2;
            }
        }
    }

    /// Connected-component labelling using 8-neighbour flood fill.
    /// Returns (labels, component count); 0 means empty.
    pub fn label_components(&self) -> (Vec<u32>, u32) {
        let n = self.mask.len();
        let mut labels = vec![0u32; n];
        let mut next = 1u32;
        let mut stack: Vec<usize> = Vec::new();
        for start in 0..n {
            if !self.mask[start] || labels[start] != 0 {
                continue;
            }
            labels[start] = next;
            stack.push(start);
            while let Some(i) = pop(&mut stack) {
                let x = (i % self.width as usize) as i64;
                let y = (i / self.width as usize) as i64;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = x + dx;
                        let ny = y + dy;
                        if nx < 0 || ny < 0 || nx >= self.width as i64 || ny >= self.height as i64 {
                            continue;
                        }
                        let ni = ny as usize * self.width as usize + nx as usize;
                        if self.mask[ni] && labels[ni] == 0 {
                            labels[ni] = next;
                            stack.push(ni);
                        }
                    }
                }
            }
            next += 1;
        }
        (labels, next - 1)
    }

    pub fn set_px(&mut self, x_nm: i64, y_nm: i64, value: bool) {
        self.set(x_nm, y_nm, value);
    }
}

fn pop(stack: &mut Vec<usize>) -> Option<usize> {
    stack.pop()
}
