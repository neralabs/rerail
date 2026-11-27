from shop.factory import login_required


@login_required
def home():
    return "home"
