pub(crate) type BuilderResult<R> = std::result::Result<R, BuilderError>;

#[derive(Debug)]
pub enum BuilderError {
    UnexpectedStructure,
    TODO_TotallyUnexpected,
}
