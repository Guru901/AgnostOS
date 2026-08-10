/// A pixel position in a framebuffer.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PixelCoord {
    x: usize,
    y: usize,
}

impl PixelCoord {
    #[must_use]
    pub const fn new(x: usize, y: usize) -> Self {
        Self { x, y }
    }

    pub(crate) const fn x(self) -> usize {
        self.x
    }
    pub(crate) const fn y(self) -> usize {
        self.y
    }
}

/// A width and height measured in pixels.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PixelSize {
    width: usize,
    height: usize,
}

impl PixelSize {
    #[must_use]
    pub const fn new(width: usize, height: usize) -> Self {
        assert!(
            width != 0 && height != 0,
            "pixel dimensions must be non-zero"
        );
        Self { width, height }
    }

    pub(crate) const fn try_new(width: usize, height: usize) -> Option<Self> {
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self { width, height })
        }
    }

    #[must_use]
    pub(crate) const fn width(self) -> usize {
        self.width
    }

    #[must_use]
    pub(crate) const fn height(self) -> usize {
        self.height
    }
}

/// The number of pixels between consecutive framebuffer rows.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Stride(usize);

impl Stride {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        assert!(value != 0, "framebuffer stride must be non-zero");
        Self(value)
    }

    pub(crate) const fn try_new(value: usize) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// A circle radius measured in pixels.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PixelRadius(usize);

impl PixelRadius {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// A vertical displacement measured in pixel rows.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PixelRows(usize);

impl PixelRows {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}
