from httpx import HTTPStatusError
from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Horizontal
from textual.screen import ModalScreen
from textual.widgets import Button, Label, Markdown, Rule, TextArea

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github import issues, pull_requests
from lazy_github.lib.logging import lg
from lazy_github.lib.messages import NewCommentCreated
from lazy_github.models.github import Issue, IssueComment, Repository, Review, ReviewComment
from lazy_github.ui.widgets.common import LazyGithubFooter

CommmentReplyTarget = ReviewComment | Review | IssueComment


class ReplyingToContainer(Container):
    def __init__(self, reply_to: CommmentReplyTarget, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        self.reply_to = reply_to

    def compose(self) -> ComposeResult:
        if isinstance(self.reply_to, (ReviewComment, IssueComment)):
            comment_time = self.reply_to.created_at.strftime("%c")
        elif self.reply_to.submitted_at:
            comment_time = self.reply_to.submitted_at.strftime("%c")
        else:
            return

        author = self.reply_to.user.login if self.reply_to.user else "Unknown"
        yield Label(f"Replying to comment from {author} at {comment_time}")
        yield Markdown(self.reply_to.body)
        yield Rule()


class NewCommentContainer(Container):
    DEFAULT_CSS = """
    #new_comment_body {
        border: blank white ;
        min-height: 40%;
        width: 100%;
        padding-top: 1;
    }

    #comment_preview {
        border: blank white;
        min-height: 40%;
        width: 100%;
        padding-top: 1;
    }

    Horizontal {
        align: center middle;
    }
    """

    def __init__(
        self,
        repo: Repository,
        issue: Issue,
        reply_to: CommmentReplyTarget | None,
        *args,
        **kwargs,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.reply_to = reply_to
        self.repo = repo
        self.issue = issue

    def compose(self) -> ComposeResult:
        if self.reply_to:
            yield ReplyingToContainer(self.reply_to)
        yield Label("New comment")
        yield TextArea(id="new_comment_body")
        yield Label("Preview")
        yield Markdown(id="comment_preview")
        with Horizontal():
            yield Button("Post Comment", id="post_comment", variant="success")
            yield Button("Cancel", id="cancel_comment", variant="error")

    @on(TextArea.Changed, "#new_comment_body")
    async def comment_updated(self, updated_message: TextArea.Changed) -> None:
        self.query_one("#comment_preview", Markdown).update(updated_message.text_area.text)

    @on(Button.Pressed, "#post_comment")
    async def post_comment(self, _: Button.Pressed) -> None:
        body = self.query_one("#new_comment_body", TextArea).text
        try:
            if isinstance(self.reply_to, ReviewComment):
                new_comment = await pull_requests.reply_to_review_comment(self.repo, self.issue, self.reply_to, body)
            else:
                new_comment = await issues.create_comment(self.issue, body)
        except HTTPStatusError:
            # TODO: We should handle the error case better here
            lg.exception(f"Error while posting comment for issue #{self.issue.number}")
            self.app.pop_screen()
        else:
            lg.info(f"Successfully posted new comment for issue #{self.issue.number}")
            self.post_message(NewCommentCreated(new_comment))

    @on(Button.Pressed, "#cancel_comment")
    def cancel_comment(self, _: Button.Pressed) -> None:
        self.app.pop_screen()


class NewCommentModal(ModalScreen[IssueComment | None]):
    DEFAULT_CSS = """
    NewCommentModal {
        align: center middle;
        content-align: center middle;
    }

    NewCommentContainer {
        width: 100;
        height: 50;
        border: thick $background 80%;
        background: $surface-lighten-3;
        margin: 1;
    }
    """

    BINDINGS = [LazyGithubBindings.CLOSE_DIALOG]

    def __init__(
        self,
        repo: Repository,
        issue: Issue,
        reply_to: CommmentReplyTarget | None,
        *args,
        **kwargs,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.reply_to = reply_to
        self.repo = repo
        self.issue = issue

    def compose(self) -> ComposeResult:
        yield NewCommentContainer(self.repo, self.issue, self.reply_to)
        yield LazyGithubFooter()

    @on(NewCommentCreated)
    def on_comment_created(self, message: NewCommentCreated) -> None:
        self.dismiss(message.comment)

    def action_close(self) -> None:
        self.dismiss(None)
