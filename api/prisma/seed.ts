import { PrismaClient, type BlogPost, type BlogComment } from "@prisma/client";
import { faker } from "@faker-js/faker";

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
      title: "Test page",
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
        author_ip: "127.0.0.1",
        author_name: faker.person.fullName(),
        author_email: faker.internet.email(),
        content: faker.lorem.paragraphs({ min: 1, max: 4 }),
        post_id: post.id,
        parent_id: parentComment?.id,
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
