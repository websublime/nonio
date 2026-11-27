class UserRepo:
    def load(self, user_id: int):
        return {"id": user_id}


class CacheRepo:
    def load(self, user_id: int):
        return None
