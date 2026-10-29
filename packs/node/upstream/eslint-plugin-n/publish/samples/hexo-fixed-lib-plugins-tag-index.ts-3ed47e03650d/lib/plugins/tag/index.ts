import moize from 'moize';
import type Hexo from '../../hexo';

const blockquoteTag = require('./blockquote');
const codeTag = require('./code');
const iframeTag = require('./iframe');
const imgTag = require('./img');
const includeCodeTag = require('./include_code');
const link = require('./link');
const postPathTag = require('./post_path');
const postLinkTag = require('./post_link');
const assetPathTag = require('./asset_path');
const assetLinkTag = require('./asset_link');
const assetImgTag = require('./asset_img');
const pullquoteTag = require('./pullquote');
const urlForTag = require('./url_for');
const fullUrlForTag = require('./full_url_for');

export default (ctx: Hexo) => {
  const { tag } = ctx.extend;

  const blockquote = blockquoteTag(ctx);

  tag.register('quote', blockquote, true);
  tag.register('blockquote', blockquote, true);

  const code = codeTag(ctx);

  tag.register('code', code, true);
  tag.register('codeblock', code, true);

  tag.register('iframe', iframeTag);

  const img = imgTag(ctx);

  tag.register('img', img);
  tag.register('image', img);

  const includeCode = includeCodeTag(ctx);

  tag.register('include_code', includeCode, {async: true});
  tag.register('include-code', includeCode, {async: true});

  tag.register('a', link);
  tag.register('link', link);
  tag.register('anchor', link);

  tag.register('post_path', postPathTag(ctx));
  tag.register('post_link', postLinkTag(ctx));

  tag.register('asset_path', assetPathTag(ctx));
  tag.register('asset_link', assetLinkTag(ctx));

  const assetImg = assetImgTag(ctx);

  tag.register('asset_img', assetImg);
  tag.register('asset_image', assetImg);

  tag.register('pullquote', pullquoteTag(ctx), true);

  tag.register('url_for', urlForTag(ctx));
  tag.register('full_url_for', fullUrlForTag(ctx));
};

// Use WeakMap to track different ctx (in case there is any)
const moized = new WeakMap();

export function postFindOneFactory(ctx) {
  if (moized.has(ctx)) {
    return moized.get(ctx);
  }

  const moizedPostFindOne = moize(createPostFindOne(ctx), {
    isDeepEqual: true,
    maxSize: 20
  });
  moized.set(ctx, moizedPostFindOne);

  return moizedPostFindOne;
}

function createPostFindOne(ctx) {
  const Post = ctx.model('Post');
  return Post.findOne.bind(Post);
}
