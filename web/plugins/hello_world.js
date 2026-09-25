export default {
    name: "Hello World Plugin",
    init: (OS) => {
        console.log("Hello World Plugin initialized!");
        
        // Add a button to the top toolbar
        const toolbar = document.querySelector('.main-header');
        if (toolbar) {
            const btn = document.createElement('button');
            btn.className = 'btn';
            btn.style.marginLeft = '8px';
            btn.style.backgroundColor = '#ff0055';
            btn.style.color = 'white';
            btn.textContent = '🌟 Plugin Button';
            btn.onclick = () => {
                alert("Hello from the dynamically loaded TypeScript/JS Plugin!");
            };
            toolbar.appendChild(btn);
        }
    }
};
