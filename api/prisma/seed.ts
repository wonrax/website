import { PrismaClient, BlogPost, BlogComment } from "@prisma/client";

const prisma = new PrismaClient();

async function main() {
  try {
    await prisma.counter.create({
      data: {
        key: "github-profile-views",
        name: "wonrax",
        count: 255,
      },
    });
  } catch {}

  await seedPosts();
}

async function seedPosts() {
  const post = await prisma.blogPost.create({
    data: {
      category: "blog",
      slug: "test",
      title: "Test title",
    },
  });

  await seedComments(post, undefined, [], 3);
}

// Add n child comments if the comment level is 1, decrease the number of child
// comments by 1 by each level
async function seedComments(
  post: BlogPost,
  parentComment?: BlogComment,
  root: number[] = [],
  n = 3
) {
  for (let i = 0; i < n; i++) {
    const newRoot = [...root, i + 1];
    const comment = await prisma.blogComment.create({
      data: {
        author_ip: "xxx.xxx.xxx.xxx",
        author_name: "Test Author",
        author_email: "test@mail.com",
        content: newRoot.join("."),
        post_id: post.id,
        parent_id: parentComment?.id,
        upvote: i,
      },
    });

    await seedComments(post, comment, newRoot, n - 1);
  }
}

main()
  .then(async () => {
    await prisma.$disconnect();
  })
  .catch(async (e) => {
    console.error(e);
    await prisma.$disconnect();
    process.exit(1);
  });
