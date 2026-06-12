-- Render-ready parsed answer blocks (JSON array of AnswerBlock), nullable.
-- NULL = legacy turn predating this column (parsed from `answer` on read).
ALTER TABLE conversation_turns ADD COLUMN blocks TEXT;
