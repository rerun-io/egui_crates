use crate::{Container, ContainerKind};

/// An identifier for a [`Tile`] in the tree, be it a [`Container`] or a pane.
///
/// This id is unique within the tree, but not across trees.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct TileId(pub u64);

impl TileId {
    #[inline]
    pub fn from_u64(n: u64) -> Self {
        Self(n)
    }

    /// Corresponding [`egui::Id`], used for tracking dragging of tiles.
    pub fn egui_id(&self, tree_id: egui::Id) -> egui::Id {
        tree_id.with(("tile", self))
    }
}

impl std::fmt::Debug for TileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

// ----------------------------------------------------------------------------

/// A tile in the tree. Either a pane (leaf) or a [`Container`] of more tiles.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Tile<Pane> {
    /// A leaf. This is where the user puts their UI, using the [`crate::Behavior`] trait.
    Pane(Pane),

    /// A container of more tiles, e.g. a horizontal layout or a tab layout.
    Container(Container),
}

impl<T> From<Container> for Tile<T> {
    #[inline]
    fn from(container: Container) -> Self {
        Self::Container(container)
    }
}

impl<Pane> Tile<Pane> {
    /// Returns `None` if this is a [`Self::Pane`].
    #[inline]
    pub fn kind(&self) -> Option<ContainerKind> {
        match self {
            Self::Pane(_) => None,
            Self::Container(container) => Some(container.kind()),
        }
    }

    #[inline]
    pub fn is_pane(&self) -> bool {
        matches!(self, Self::Pane(_))
    }

    #[inline]
    pub fn is_container(&self) -> bool {
        matches!(self, Self::Container(_))
    }

    #[inline]
    pub fn container_kind(&self) -> Option<ContainerKind> {
        match self {
            Self::Pane(_) => None,
            Self::Container(container) => Some(container.kind()),
        }
    }
}
