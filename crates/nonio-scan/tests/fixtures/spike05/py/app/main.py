from fastapi import FastAPI

from .services import create_user
from .repo import UserRepo

app = FastAPI()


@app.post("/users")
def post_user(name: str):
    return create_user(name)


@app.get("/users/{user_id}")
def get_user(user_id: int):
    repo = UserRepo()
    return repo.load(user_id)


def notify(client, text: str):
    # `client` is a parameter: it must never resolve to the module app/client.py
    return client.send(text)
