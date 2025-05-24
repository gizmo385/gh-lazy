from textual import work
from textual.app import ComposeResult
from textual.containers import Container
from textual.content import Content
from textual.widgets import Collapsible, Label, Markdown

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.pull_requests import ReviewCommentNode
from lazy_github.lib.messages import NewCommentCreated
from lazy_github.models.github import FullPullRequest, Issue, IssueComment, Review, ReviewComment, ReviewState
from lazy_github.ui.screens.new_comment import NewCommentModal


class IssueCommentContainer(Container, can_focus=True):
    DEFAULT_CSS = """
    IssueCommentContainer {
        height: auto;
        border-left: solid $secondary-background;
        margin-left: 1;
        margin-bottom: 1;
    }

    IssueCommentContainer:focus-within {
        border: dashed $success;
    }

    .comment-author {
        color: $text-muted;
        margin-left: 1;
        margin-bottom: 1;
    }

    Markdown {
        margin-left: 1;
        margin-bottom: 0;
        padding-bottom: 0;
    }

    """

    BINDINGS = [LazyGithubBindings.REPLY_TO_COMMENT]

    def __init__(self, issue: Issue, comment: IssueComment) -> None:
        super().__init__()
        self.issue = issue
        self.comment = comment

    def compose(self) -> ComposeResult:
        comment_time = self.comment.created_at.strftime("%c")
        author = self.comment.user.login if self.comment.user else "Unknown"
        yield Markdown(self.comment.body)
        yield Label(f"{author} • {comment_time}", classes="comment-author")

    @work
    async def reply_to_comment_flow(self) -> None:
        reply_comment = await self.app.push_screen_wait(NewCommentModal(self.issue.repo, self.issue, self.comment))
        if reply_comment is not None:
            self.post_message(NewCommentCreated(reply_comment))

    def action_reply_to_individual_comment(self) -> None:
        self.reply_to_comment_flow()


class ReviewConversation(Container):
    DEFAULT_CSS = """
    ReviewConversation {
        height: auto;
        border-left: solid $secondary-background;
        margin-bottom: 1;
    }
    """

    def __init__(self, pr: FullPullRequest, root_conversation_node: ReviewCommentNode) -> None:
        super().__init__()
        self.pr = pr
        self.root_conversation_node = root_conversation_node

    def _flatten_comments(self, root: ReviewCommentNode) -> list[ReviewComment]:
        result = [root.comment]
        for child in root.children:
            result.extend(self._flatten_comments(child))
        return result

    def compose(self) -> ComposeResult:
        for comment in self._flatten_comments(self.root_conversation_node):
            yield IssueCommentContainer(self.pr, comment)


class ReviewContainer(Collapsible, can_focus=True):
    DEFAULT_CSS = """
    ReviewContainer {
        height: auto;
    }

    ReviewContainer:focus-within {
        border: solid $success-lighten-3;
    }
    """
    BINDINGS = [LazyGithubBindings.REPLY_TO_REVIEW]

    def __init__(self, pr: FullPullRequest, review: Review, hierarchy: dict[int, ReviewCommentNode]) -> None:
        super().__init__()
        self.pr = pr
        self.review = review
        self.hierarchy = hierarchy

    def compose(self) -> ComposeResult:
        if self.review.state == ReviewState.APPROVED:
            review_state_text = "[greenyellow]Approved[/]"
        elif self.review.state == ReviewState.CHANGES_REQUESTED:
            review_state_text = "[red]Changes Requested[/red]"
        else:
            review_state_text = self.review.state.title()

        yield Label(Content.from_markup(f"Review from {self.review.user.login} ({review_state_text})"))

        if self.review.body:
            yield Markdown(self.review.body)

        for comment in self.review.comments:
            if comment_node := self.hierarchy.get(comment.id):
                yield ReviewConversation(self.pr, comment_node)

    def action_reply_to_review(self) -> None:
        self.app.push_screen(NewCommentModal(self.pr.repo, self.pr, self.review))
